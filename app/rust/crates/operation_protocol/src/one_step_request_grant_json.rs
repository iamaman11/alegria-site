//! The only raw JSON ingress for an opaque preauthorized Start request.
//! Keep wire parsing out of the pure grant/ACL/budget verifier. Neither this
//! adapter nor a successful parse grants Start rights or issuer authority.

use crate::one_step_request_grant_v1::{
    canonical_non_nil_uuid, OneStepRequestGrantErrorV1, OpaqueOneStepRequestLocatorV1,
};

const MAX_LOCATOR_BYTES: usize = 80;

/// Parse the only client-supplied field as one exact canonical JSON object.
/// No tenant, run, context, policy, route, Temporal endpoint, path or source
/// can pass through this adapter; the issuer independently looks them up.
pub fn decode_opaque_one_step_request_locator_v1(
    bytes: &[u8],
) -> Result<OpaqueOneStepRequestLocatorV1, OneStepRequestGrantErrorV1> {
    if bytes.is_empty() || bytes.len() > MAX_LOCATOR_BYTES {
        return Err(OneStepRequestGrantErrorV1::InvalidLocator);
    }
    let locator: OpaqueOneStepRequestLocatorV1 =
        serde_json::from_slice(bytes).map_err(|_| OneStepRequestGrantErrorV1::InvalidLocator)?;
    if !canonical_non_nil_uuid(&locator.request_id)
        || serde_json::to_vec(&locator).ok().as_deref() != Some(bytes)
    {
        return Err(OneStepRequestGrantErrorV1::InvalidLocator);
    }
    Ok(locator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_rejects_unexpected_control_fields() {
        let locator = OpaqueOneStepRequestLocatorV1 {
            request_id: "e0813cac-2277-454e-980a-2e08110c1a08".into(),
        };
        let valid = serde_json::to_vec(&locator).unwrap();
        assert!(decode_opaque_one_step_request_locator_v1(&valid).is_ok());
        let mut extra = serde_json::to_value(&locator).unwrap();
        extra["tenant_id"] = "tenant-attacker".into();
        assert!(serde_json::from_value::<OpaqueOneStepRequestLocatorV1>(extra).is_err());
        let mut extra = serde_json::to_value(&locator).unwrap();
        extra["operator_intent_path"] = "/tmp/intent".into();
        assert!(serde_json::from_value::<OpaqueOneStepRequestLocatorV1>(extra).is_err());
    }
}
