use super::*;
use crate::operations::{ClassifySourcePage, PureAcquisitionOperation};
use crate::source_relevance::PageMode;

const SOURCE_HTML: &str = "<html><head><title>Captured page</title></head><body>\
    <nav>Cookie menu directory</nav>\
    <main><h1>Visa documents</h1><p>Official requirements for visitors.</p>\
    <script>const hidden = 'cookie directory';</script>\
    <div hidden>Directory navigation</div><div style='display: none'>Cookie sign-in</div>\
    <noscript>Cookie utility</noscript></main>\
    <footer>All countries menu</footer></body></html>";

fn snapshot(html: &str) -> CapturedHtmlPageV1 {
    CapturedHtmlPageV1 {
        page_id: 42,
        url: "https://example.org/old-url".into(),
        final_url: Some("https://example.org/final-url".into()),
        content_type: Some("text/html; charset=UTF-8".into()),
        status_code: Some(200),
        raw_html: Some(html.into()),
        raw_html_bytes: Some(i32::try_from(html.len()).unwrap()),
        content_hash: Some(content_hash_v1(html)),
    }
}

#[test]
fn trusted_raw_html_derives_exact_typed_bytes_and_ignores_navigation_and_scripts() {
    let derived = classify_source_page_payload_from_pinned_html_v1(&snapshot(SOURCE_HTML))
        .expect("trusted HTML is accepted");
    assert_eq!(derived.input.url, "https://example.org/final-url");
    assert_eq!(derived.input.title, "Captured page");
    assert_eq!(
        derived.input.raw_text,
        "Visa documents Official requirements for visitors."
    );
    assert_eq!(
        derived.payload,
        br#"{"url":"https://example.org/final-url","title":"Captured page","raw_text":"Visa documents Official requirements for visitors."}"#
    );
    assert_eq!(derived.raw_content_hash, content_hash_v1(SOURCE_HTML));
    assert_eq!(derived.payload_hash, blake3_hex(&derived.payload));
    assert_ne!(derived.payload_hash, derived.raw_content_hash);
    assert_eq!(
        ClassifySourcePage::evaluate(&derived.input).page_mode,
        PageMode::ContentPage,
        "script/footer should not force utility/directory classification"
    );
    let repeated = classify_source_page_payload_from_pinned_html_v1(&snapshot(SOURCE_HTML))
        .expect("repeated transformation is deterministic");
    assert_eq!(derived.payload, repeated.payload);
    assert_eq!(derived.payload_hash, repeated.payload_hash);
}

#[test]
fn rejects_missing_forged_or_incomplete_raw_ingress_metadata() {
    let mut bad = snapshot(SOURCE_HTML);
    bad.page_id = 0;
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&bad).unwrap_err(),
        TrustedSourceTransformErrorV1::InvalidSnapshotIdentity
    );

    let mut bad = snapshot(SOURCE_HTML);
    bad.content_hash = Some("0".repeat(64));
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&bad).unwrap_err(),
        TrustedSourceTransformErrorV1::RawContentHashMismatch
    );

    let mut bad = snapshot(SOURCE_HTML);
    bad.raw_html_bytes = None;
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&bad).unwrap_err(),
        TrustedSourceTransformErrorV1::InvalidRawByteCount
    );

    let mut bad = snapshot(SOURCE_HTML);
    bad.raw_html = None;
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&bad).unwrap_err(),
        TrustedSourceTransformErrorV1::MissingRawHtml
    );

    let mut bad = snapshot(SOURCE_HTML);
    bad.status_code = Some(404);
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&bad).unwrap_err(),
        TrustedSourceTransformErrorV1::UnsuccessfulCapture
    );

    let mut bad = snapshot(SOURCE_HTML);
    bad.content_type = Some("application/pdf".into());
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&bad).unwrap_err(),
        TrustedSourceTransformErrorV1::UnsupportedSourceType
    );

    let mut bad = snapshot(SOURCE_HTML);
    bad.final_url = Some("file:///etc/passwd".into());
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&bad).unwrap_err(),
        TrustedSourceTransformErrorV1::InvalidSourceUrl
    );

    for bad_url in [
        "https://username:secret@example.org/",
        "https://example.org/fragment#value",
        "https://example.org/a b",
        "https://",
    ] {
        let mut bad = snapshot(SOURCE_HTML);
        bad.final_url = Some(bad_url.into());
        assert_eq!(
            classify_source_page_payload_from_pinned_html_v1(&bad).unwrap_err(),
            TrustedSourceTransformErrorV1::InvalidSourceUrl
        );
    }
}

#[test]
fn refuses_empty_visible_body_and_oversize_html_without_truncation() {
    let only_hidden = "<html><head><title>Source title</title></head>\
        <body><script>visible only to JS</script><nav>Navigation</nav></body></html>";
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&snapshot(only_hidden)).unwrap_err(),
        TrustedSourceTransformErrorV1::NoVisibleContent
    );
    let oversized = format!(
        "<html><body><main>{}</main></body></html>",
        "a".repeat(2_097_152)
    );
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&snapshot(&oversized)).unwrap_err(),
        TrustedSourceTransformErrorV1::RawHtmlTooLarge
    );
}

#[test]
fn utf8_captured_byte_count_is_not_character_count() {
    let html =
        "<html><head><title>Кухни</title></head><body><main>Кухни на заказ</main></body></html>";
    let mut page = snapshot(html);
    page.raw_html_bytes = Some(i32::try_from(html.chars().count()).unwrap());
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&page).unwrap_err(),
        TrustedSourceTransformErrorV1::InvalidRawByteCount
    );
    page.raw_html_bytes = Some(i32::try_from(html.len()).unwrap());
    let derived = classify_source_page_payload_from_pinned_html_v1(&page).unwrap();
    assert_eq!(derived.input.title, "Кухни");
    assert_eq!(derived.input.raw_text, "Кухни на заказ");
}

#[test]
fn input_json_budget_rejects_escape_expansion_instead_of_truncating() {
    let html = format!(
        "<html><head><title>Valid</title></head><body><main>{}</main></body></html>",
        "\"".repeat(262_000)
    );
    assert_eq!(
        classify_source_page_payload_from_pinned_html_v1(&snapshot(&html)).unwrap_err(),
        TrustedSourceTransformErrorV1::WirePayloadTooLarge
    );
}

#[test]
fn stored_untrusted_title_cannot_override_captured_html() {
    // No title field exists on CapturedHtmlPageV1: only observed markup may
    // supply one. Without <title>, the captured H1 is the deterministic fallback.
    let html = "<html><body><main><h1>Observed H1</h1><p>Body.</p></main></body></html>";
    let derived = classify_source_page_payload_from_pinned_html_v1(&snapshot(html)).unwrap();
    assert_eq!(derived.input.title, "Observed H1");
}
