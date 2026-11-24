//! Pure readiness policy for the immutable publication artifact.
//! Filesystem paths, deployment URLs, and manifest persistence are adapters.

#[derive(Debug, Clone, Copy)]
pub struct MaterializationFields<'a> {
    pub page_node_key: &'a str,
    pub revision_id: &'a str,
    pub canonical_url_path: &'a str,
    pub output_dir: &'a str,
    pub base_url: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingMaterializationField {
    PageNodeKey,
    RevisionId,
    CanonicalUrlPath,
    OutputDir,
    BaseUrl,
}

impl MissingMaterializationField {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::PageNodeKey => "missing_page_node_key",
            Self::RevisionId => "missing_revision_id",
            Self::CanonicalUrlPath => "missing_canonical_url_path",
            Self::OutputDir => "missing_output_dir",
            Self::BaseUrl => "missing_base_url",
        }
    }
}

impl MaterializationFields<'_> {
    pub fn missing_required(self) -> Vec<MissingMaterializationField> {
        let mut missing = Vec::new();
        if self.page_node_key.trim().is_empty() {
            missing.push(MissingMaterializationField::PageNodeKey);
        }
        if self.revision_id.trim().is_empty() {
            missing.push(MissingMaterializationField::RevisionId);
        }
        if self.canonical_url_path.trim().is_empty() {
            missing.push(MissingMaterializationField::CanonicalUrlPath);
        }
        if self.output_dir.trim().is_empty() {
            missing.push(MissingMaterializationField::OutputDir);
        }
        if self.base_url.trim().is_empty() {
            missing.push(MissingMaterializationField::BaseUrl);
        }
        missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_all_missing_inputs_in_stable_wire_order() {
        let reasons = MaterializationFields {
            page_node_key: "",
            revision_id: " ",
            canonical_url_path: "\n",
            output_dir: "",
            base_url: "",
        }
        .missing_required()
        .iter()
        .map(|field| field.reason_code())
        .collect::<Vec<_>>();
        assert_eq!(
            reasons,
            vec![
                "missing_page_node_key",
                "missing_revision_id",
                "missing_canonical_url_path",
                "missing_output_dir",
                "missing_base_url",
            ]
        );
    }

    #[test]
    fn accepts_fully_resolved_input() {
        assert!(MaterializationFields {
            page_node_key: "page-1",
            revision_id: "revision-1",
            canonical_url_path: "/visa/spain",
            output_dir: "/tmp/render",
            base_url: "https://example.test",
        }
        .missing_required()
        .is_empty());
    }
}
