//! Trusted JSON authoring wire adapter for the existing Site Lab operation catalog.
//!
//! A recipe supplies JSON values, never Rust type names or Task Queues. An
//! installed owner binds those values to its concrete serde DTO or registers
//! an explicit remote ValidateOnly requirement. The generic DAG remains
//! unaware of provider DTOs and never grants execution admission here.

use std::collections::BTreeMap;

use operation_protocol::{
    owner_content_hash, ContractCatalog, ContractHash, Operation, OperationRef, PlanError,
};
use seo_domain::extraction::operations::{
    CanonicalizeMentions, EditorialTopics, EntitySpans, LayerRoute, OperationalEntities,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

type DecodeParameters = Box<dyn Fn(&Value) -> Result<Vec<u8>, String> + Send + Sync>;

/// A provider DTO may not use `deny_unknown_fields`, or a nested serde DTO
/// may silently drop a newly added provider option. Fail closed if any value
/// supplied by the author disappears or changes on typed round-trip. Fields
/// added by DTO defaults are permitted; author-provided settings are never
/// silently ignored by the generic composer.
fn preserves_authored_fields(authored: &Value, normalized: &Value) -> bool {
    match (authored, normalized) {
        (Value::Object(given), Value::Object(typed)) => given.iter().all(|(key, value)| {
            typed
                .get(key)
                .is_some_and(|typed_value| preserves_authored_fields(value, typed_value))
        }),
        (Value::Array(given), Value::Array(typed)) => {
            given.len() == typed.len()
                && given
                    .iter()
                    .zip(typed)
                    .all(|(given, typed)| preserves_authored_fields(given, typed))
        }
        _ => authored == normalized,
    }
}

struct ParameterDescriptor {
    contract_hash: ContractHash,
    schema: Value,
    schema_hash: String,
    local_decoder: Option<DecodeParameters>,
}

/// Authoring-only JSON envelope. The graph planner never decodes provider
/// settings: the installed owner codec is the sole DTO validation boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecipeParameters(Value);

impl Default for RecipeParameters {
    fn default() -> Self {
        Self(json!({}))
    }
}

impl RecipeParameters {
    pub fn is_empty(&self) -> bool {
        self.0.as_object().is_some_and(|map| map.is_empty())
    }
}

impl From<Value> for RecipeParameters {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

/// One logical trusted operation catalog, with optional owner-owned parameter
/// contracts. Registering an operation schema does NOT register an executable
/// owner route or prove that provider credentials are available.
pub struct SiteLabCatalog {
    contracts: ContractCatalog,
    parameters: BTreeMap<OperationRef, ParameterDescriptor>,
}

#[derive(Debug, Error)]
pub enum ParameterError {
    #[error("parameter contract references an unregistered operation")]
    UnknownOperation,
    #[error("parameter contract fingerprint differs from its installed operation")]
    MismatchedContract,
    #[error("parameter contract already registered for this operation")]
    Duplicate,
    #[error("parameter schema must describe an object")]
    InvalidSchema,
    #[error("registered parameter schema is unavailable for this operation")]
    MissingSchema,
    #[error("parameters cannot be decoded into the installed owner's Rust DTO")]
    InvalidParameters,
    #[error("parameter JSON cannot be encoded")]
    Encoding,
    #[error("registered operation contract is invalid")]
    InvalidContract(#[from] PlanError),
}

#[derive(Debug, Clone, Serialize)]
pub struct ParameterDescription {
    pub json_schema: Value,
    pub schema_hash: String,
    pub verification: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckedParameters {
    pub digest: String,
    /// Only a codec compiled from the owner's Rust DTO may assert this.
    pub verification: &'static str,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoParameters {}

impl SiteLabCatalog {
    pub fn new(contracts: ContractCatalog) -> Self {
        Self {
            contracts,
            parameters: BTreeMap::new(),
        }
    }

    pub fn catalog(&self) -> &ContractCatalog {
        &self.contracts
    }

    pub fn register_contract(
        &mut self,
        contract: operation_protocol::OperationContract,
    ) -> Result<(), ParameterError> {
        self.contracts.register_contract(contract)?;
        Ok(())
    }

    /// Call only for a trusted installed owner, never from user-authored JSON.
    /// The schema is discoverable; the concrete serde DTO validates values.
    pub fn register_local<T>(
        &mut self,
        operation: &OperationRef,
        contract_hash: &ContractHash,
        schema: Value,
    ) -> Result<(), ParameterError>
    where
        T: DeserializeOwned + Serialize + Send + Sync + 'static,
    {
        let descriptor = self.descriptor(operation, contract_hash, schema)?;
        self.parameters.insert(
            operation.clone(),
            ParameterDescriptor {
                local_decoder: Some(Box::new(|value: &Value| {
                    let decoded: T = serde_json::from_value(value.clone())
                        .map_err(|_| "invalid typed parameters".to_owned())?;
                    let normalized = serde_json::to_value(&decoded)
                        .map_err(|_| "parameter normalization failed".to_owned())?;
                    if !preserves_authored_fields(value, &normalized) {
                        return Err("typed parameters dropped or changed authored settings".to_owned());
                    }
                    serde_json::to_vec(&decoded).map_err(|_| "parameter encoding failed".to_owned())
                })),
                ..descriptor
            },
        );
        Ok(())
    }

    /// A separately deployed owner may publish a trusted schema without
    /// linking its Rust DTO into this application. Its bytes still require a
    /// real owner ValidateOnly Child result before any effect can start.
    pub fn register_remote(
        &mut self,
        operation: &OperationRef,
        contract_hash: &ContractHash,
        schema: Value,
    ) -> Result<(), ParameterError> {
        let descriptor = self.descriptor(operation, contract_hash, schema)?;
        self.parameters.insert(operation.clone(), descriptor);
        Ok(())
    }

    fn descriptor(
        &self,
        operation: &OperationRef,
        contract_hash: &ContractHash,
        schema: Value,
    ) -> Result<ParameterDescriptor, ParameterError> {
        let contract = self
            .contracts
            .contract(operation)
            .ok_or(ParameterError::UnknownOperation)?;
        if contract.fingerprint()? != *contract_hash {
            return Err(ParameterError::MismatchedContract);
        }
        if self.parameters.contains_key(operation) {
            return Err(ParameterError::Duplicate);
        }
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(ParameterError::InvalidSchema);
        }
        let encoded = serde_json::to_vec(&schema).map_err(|_| ParameterError::Encoding)?;
        Ok(ParameterDescriptor {
            contract_hash: contract_hash.clone(),
            schema_hash: owner_content_hash(&encoded),
            schema,
            local_decoder: None,
        })
    }

    pub fn describe(&self, operation: &OperationRef) -> Option<ParameterDescription> {
        let descriptor = self.parameters.get(operation)?;
        Some(ParameterDescription {
            json_schema: descriptor.schema.clone(),
            schema_hash: descriptor.schema_hash.clone(),
            verification: if descriptor.local_decoder.is_some() {
                "typed_local"
            } else {
                "requires_owner_validate_only"
            },
        })
    }

    pub fn validate(
        &self,
        operation: &OperationRef,
        parameters: &RecipeParameters,
    ) -> Result<CheckedParameters, ParameterError> {
        let value = &parameters.0;
        let descriptor = self
            .parameters
            .get(operation)
            .ok_or(ParameterError::MissingSchema)?;
        let contract = self
            .contracts
            .contract(operation)
            .ok_or(ParameterError::UnknownOperation)?;
        if contract.fingerprint()? != descriptor.contract_hash {
            return Err(ParameterError::MismatchedContract);
        }
        if !value.is_object() {
            return Err(ParameterError::InvalidParameters);
        }
        let bytes = match &descriptor.local_decoder {
            Some(decode) => decode(value).map_err(|_| ParameterError::InvalidParameters)?,
            None => serde_json::to_vec(value).map_err(|_| ParameterError::Encoding)?,
        };
        // Bind both the exact trusted schema and canonical parameter bytes.
        // The live owner must independently check the typed command/receipt.
        let digest = owner_content_hash(
            &serde_json::to_vec(&(&descriptor.schema_hash, &bytes))
                .map_err(|_| ParameterError::Encoding)?,
        );
        Ok(CheckedParameters {
            digest,
            verification: if descriptor.local_decoder.is_some() {
                "typed_local"
            } else {
                "requires_owner_validate_only"
            },
        })
    }
}

/// These five known in-process pure semantic operations have no configurable
/// command parameters. Their *artifact inputs* retain their concrete DTOs.
/// Do not assume every future pure semantic operation has no parameters: its
/// independently registered owner must explicitly publish its own schema.
pub fn installed_semantic_lab_catalog(
    contracts: ContractCatalog,
) -> Result<SiteLabCatalog, ParameterError> {
    let mut installed = SiteLabCatalog::new(contracts);
    let known_operations = [
        LayerRoute::contract().reference,
        EntitySpans::contract().reference,
        CanonicalizeMentions::contract().reference,
        EditorialTopics::contract().reference,
        OperationalEntities::contract().reference,
    ];
    for operation in known_operations {
        let fingerprint = installed
            .catalog()
            .contract(&operation)
            .ok_or(ParameterError::UnknownOperation)?
            .fingerprint()?;
        installed.register_local::<NoParameters>(
            &operation,
            &fingerprint,
            json!({ "type": "object", "additionalProperties": false }),
        )?;
    }
    Ok(installed)
}
