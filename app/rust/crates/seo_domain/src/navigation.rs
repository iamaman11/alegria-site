//! Pure presentation policy for navigation. Persistence owns neither labels nor
//! the meaning of page types; callers may compose navigation independently of
//! the workflow that produced the pages.

/// A human-readable label from a canonical slug or path segment.
pub fn label_from_slug(value: &str) -> String {
    let label = value
        .trim_matches('/')
        .split(['-', '_', '/', ':'])
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        "Page".to_string()
    } else {
        label
    }
}

/// The page-type-specific title policy is independent of SQL and of the
/// creation sequence. Slugs and paths are data, not orchestration steps.
pub fn label_for_page(
    page_type_key: &str,
    canonical_slug: &str,
    canonical_url_path: &str,
) -> String {
    let source = if canonical_slug.trim().is_empty() {
        canonical_url_path
            .trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("page")
    } else {
        canonical_slug
    };
    let mut label = label_from_slug(source);
    match page_type_key {
        "country_hub_page" if !label.to_ascii_lowercase().contains("visa") => {
            label.push_str(" Visa");
        }
        "fee_page" if !label.to_ascii_lowercase().contains("fee") => label.push_str(" Fees"),
        "timeline_page" if !label.to_ascii_lowercase().contains("time") => {
            label.push_str(" Timeline");
        }
        _ => {}
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_policy_is_independent_of_storage() {
        assert_eq!(
            label_from_slug("travel-documents:requirements"),
            "Travel Documents Requirements"
        );
        assert_eq!(label_from_slug("///"), "Page");
    }

    #[test]
    fn page_type_suffix_is_applied_once() {
        assert_eq!(
            label_for_page("country_hub_page", "poland", "/unused"),
            "Poland Visa"
        );
        assert_eq!(
            label_for_page("country_hub_page", "poland-visa", "/unused"),
            "Poland Visa"
        );
        assert_eq!(
            label_for_page("fee_page", "", "/de/consular-fees/"),
            "Consular Fees"
        );
        assert_eq!(
            label_for_page("timeline_page", "", "/steps/"),
            "Steps Timeline"
        );
        assert_eq!(label_for_page("custom_page", "guide", "/steps/"), "Guide");
    }
}
