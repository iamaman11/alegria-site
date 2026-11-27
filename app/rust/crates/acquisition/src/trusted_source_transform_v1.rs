//! Acquisition-owned, deterministic HTML snapshot → strict pure-operation input.
//!
//! This policy does **not** fetch pages, grant authority or create source
//! attestations. Its caller must load `raw.pages` through the independent,
//! tenant-authenticated, SELECT-only source-issuer credential. The issuer must
//! persist the exact returned payload digest alongside the captured raw hash;
//! Temporal orchestration never links this HTML parser or interprets its JSON.
//!
//! Revision V1 supports a single pinned HTML page for ACQ-01 and ACQ-02.
//! Unknown source formats fail closed instead of silently producing evidence.

use ego_tree::NodeRef;
use scraper::{node::Node, Html, Selector};
use url::Url;

use crate::operations::{
    ClassifySourcePageInput, DomBlockRelevanceInput, StrictDomBlockInput,
};
use crate::source_relevance::BlockRole;
use operation_protocol::DAG_MAX_INPUT_BYTES;
use primitives::hash::{blake3_hex, content_hash_v1};

const MAX_CAPTURED_HTML_BYTES_V1: usize = 2 * 1024 * 1024;
const MAX_VISIBLE_TEXT_BYTES_V1: usize = 256 * 1024;
const MAX_TITLE_BYTES_V1: usize = 4 * 1024;
const MAX_DOM_BLOCKS_V1: usize = 128;

/// Exact subset of a captured `raw.pages` row used by the source issuer.
/// There is deliberately no free-form, caller-authored title or raw text.
#[derive(Debug, Clone)]
pub struct CapturedHtmlPageV1 {
    pub page_id: i64,
    pub url: String,
    pub final_url: Option<String>,
    pub content_type: Option<String>,
    pub status_code: Option<i16>,
    pub raw_html: Option<String>,
    pub raw_html_bytes: Option<i32>,
    pub content_hash: Option<String>,
}

/// Stable, redacted error codes. Do not expose source HTML, credentials or
/// tenant identifiers in an operator-facing diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedSourceTransformErrorV1 {
    InvalidSnapshotIdentity,
    UnsupportedSourceType,
    UnsuccessfulCapture,
    MissingRawHtml,
    RawHtmlTooLarge,
    InvalidRawByteCount,
    RawContentHashMismatch,
    InvalidSourceUrl,
    NoVisibleContent,
    VisibleContentTooLarge,
    TitleTooLarge,
    TooManyDomBlocks,
    WirePayloadTooLarge,
    WireSerializationFailed,
}

impl std::fmt::Display for TrustedSourceTransformErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "acquisition_source_transform_{self:?}")
    }
}

impl std::error::Error for TrustedSourceTransformErrorV1 {}

#[derive(Debug, Clone)]
pub struct DerivedClassifySourcePageV1 {
    /// Strict revision-one owner input, derived only from the captured HTML.
    pub input: ClassifySourcePageInput,
    /// Exact canonical JSON bytes to admit to the ACQ-01 operation.
    pub payload: Vec<u8>,
    /// BLAKE3 of `payload`, for the separately issued SQL source record.
    pub payload_hash: String,
    /// BLAKE3 of the original HTML's UTF-8 bytes, not the JSON input.
    pub raw_content_hash: String,
}

#[derive(Debug, Clone)]
pub struct DerivedDomBlockRelevanceV1 {
    pub input: DomBlockRelevanceInput,
    pub payload: Vec<u8>,
    pub payload_hash: String,
    pub raw_content_hash: String,
}

fn is_http_source_url(url: &str) -> bool {
    Url::parse(url).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https")
            && parsed.has_host()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.fragment().is_none()
            && parsed.as_str() == url
    })
}

fn is_html_content_type(value: &str) -> bool {
    let mime = value.split(';').next().unwrap_or("").trim();
    mime.eq_ignore_ascii_case("text/html") || mime.eq_ignore_ascii_case("html")
}

fn skipped_element(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "template"
            | "svg"
            | "iframe"
            | "canvas"
            | "noscript"
            | "nav"
            | "aside"
            | "footer"
            | "form"
            | "button"
            | "select"
            | "option"
    )
}

fn hidden_inline_style(style: Option<&str>) -> bool {
    let Some(style) = style else {
        return false;
    };
    let compact = style
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    compact
        .split(';')
        .any(|declaration| declaration == "display:none" || declaration == "visibility:hidden")
}

fn collect_visible_text_with_root(
    node: NodeRef<'_, Node>,
    include_landmark_root: bool,
) -> Result<String, TrustedSourceTransformErrorV1> {
    // Iterative DOM walk: untrusted deeply nested HTML must not exhaust the
    // issuer's Rust call stack. Reject excess text instead of truncating it.
    let mut pending = vec![(node, true)];
    let mut parts = Vec::new();
    let mut total_bytes = 0usize;
    while let Some((current, root)) = pending.pop() {
        match current.value() {
            Node::Element(element) => {
                if (skipped_element(element.name()) && !(root && include_landmark_root))
                    || element.attr("hidden").is_some()
                    || element.attr("inert").is_some()
                    || element.attr("aria-hidden") == Some("true")
                    || hidden_inline_style(element.attr("style"))
                {
                    continue;
                }
            }
            Node::Text(value) => {
                let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
                if !normalized.is_empty() {
                    total_bytes = total_bytes
                        .checked_add(normalized.len() + usize::from(!parts.is_empty()))
                        .ok_or(TrustedSourceTransformErrorV1::VisibleContentTooLarge)?;
                    if total_bytes > MAX_VISIBLE_TEXT_BYTES_V1 {
                        return Err(TrustedSourceTransformErrorV1::VisibleContentTooLarge);
                    }
                    parts.push(normalized);
                }
            }
            _ => {}
        }
        let mut children = current.children().collect::<Vec<_>>();
        children.reverse();
        pending.extend(children.into_iter().map(|child| (child, false)));
    }
    Ok(parts.join(" "))
}

fn collect_visible_text(node: NodeRef<'_, Node>) -> Result<String, TrustedSourceTransformErrorV1> {
    collect_visible_text_with_root(node, false)
}

fn first_non_empty_text(
    document: &Html,
    selector: &Selector,
) -> Result<Option<String>, TrustedSourceTransformErrorV1> {
    for element in document.select(selector) {
        let node = document
            .tree
            .get(element.id())
            .ok_or(TrustedSourceTransformErrorV1::NoVisibleContent)?;
        let visible = collect_visible_text(node)?;
        if !visible.is_empty() {
            return Ok(Some(visible));
        }
    }
    Ok(None)
}

fn verified_raw_html_v1(
    source: &CapturedHtmlPageV1,
) -> Result<(&str, String), TrustedSourceTransformErrorV1> {
    use TrustedSourceTransformErrorV1 as Error;

    if source.page_id <= 0 {
        return Err(Error::InvalidSnapshotIdentity);
    }
    let content_type = source
        .content_type
        .as_deref()
        .ok_or(Error::UnsupportedSourceType)?;
    if !is_html_content_type(content_type) {
        return Err(Error::UnsupportedSourceType);
    }
    if !matches!(source.status_code, Some(200..=299)) {
        return Err(Error::UnsuccessfulCapture);
    }
    let raw_html = source.raw_html.as_deref().ok_or(Error::MissingRawHtml)?;
    if raw_html.is_empty() {
        return Err(Error::MissingRawHtml);
    }
    if raw_html.len() > MAX_CAPTURED_HTML_BYTES_V1 {
        return Err(Error::RawHtmlTooLarge);
    }
    if source.raw_html_bytes != i32::try_from(raw_html.len()).ok() {
        return Err(Error::InvalidRawByteCount);
    }
    let raw_content_hash = content_hash_v1(raw_html);
    if source.content_hash.as_deref() != Some(raw_content_hash.as_str()) {
        return Err(Error::RawContentHashMismatch);
    }
    let url = source.final_url.as_deref().unwrap_or(&source.url);
    if !is_http_source_url(url) {
        return Err(Error::InvalidSourceUrl);
    }

    Ok((raw_html, raw_content_hash))
}

fn derive_classifier_input_v1(
    source: &CapturedHtmlPageV1,
) -> Result<(ClassifySourcePageInput, String), TrustedSourceTransformErrorV1> {
    use TrustedSourceTransformErrorV1 as Error;
    let (raw_html, raw_content_hash) = verified_raw_html_v1(source)?;
    let url = source.final_url.as_deref().unwrap_or(&source.url);
    let document = Html::parse_document(raw_html);
    let title_selector = Selector::parse("head title").expect("static CSS selector");
    let h1_selector = Selector::parse("h1").expect("static CSS selector");
    let title = match first_non_empty_text(&document, &title_selector)? {
        Some(title) => title,
        None => first_non_empty_text(&document, &h1_selector)?.unwrap_or_default(),
    };
    if title.len() > MAX_TITLE_BYTES_V1 {
        return Err(Error::TitleTooLarge);
    }

    // Only the document's article body can supply classified text. Menus,
    // scripts, hidden templates and footer boilerplate never override it.
    let main_selector = Selector::parse("main").expect("static CSS selector");
    let body_selector = Selector::parse("body").expect("static CSS selector");
    let scope = document
        .select(&main_selector)
        .next()
        .or_else(|| document.select(&body_selector).next())
        .ok_or(Error::NoVisibleContent)?;
    let scope_node = document
        .tree
        .get(scope.id())
        .ok_or(Error::NoVisibleContent)?;
    let raw_text = collect_visible_text(scope_node)?;
    if raw_text.is_empty() {
        return Err(Error::NoVisibleContent);
    }
    if raw_text.len() > MAX_VISIBLE_TEXT_BYTES_V1 {
        return Err(Error::VisibleContentTooLarge);
    }
    Ok((
        ClassifySourcePageInput {
            url: url.to_string(),
            title,
            raw_text,
        },
        raw_content_hash,
    ))
}

/// Derive a bounded set of observable semantic landmarks from exactly the
/// same tenant-pinned HTML as ACQ-01. IDs come from the stored page ID and
/// deterministic selected-block order, never from caller-authored element IDs or JSON. This is a
/// raw block score, not permission to extract: ACQ-01's page gate still owns
/// that decision in a later composition.
pub fn dom_block_relevance_payload_from_pinned_html_v1(
    source: &CapturedHtmlPageV1,
) -> Result<DerivedDomBlockRelevanceV1, TrustedSourceTransformErrorV1> {
    use TrustedSourceTransformErrorV1 as Error;
    let (raw_html, raw_content_hash) = verified_raw_html_v1(source)?;
    let document = Html::parse_document(raw_html);
    let landmarks = Selector::parse(
        "body > nav, body > header, body > main, body > article, body > section, body > aside, body > footer, body > form",
    )
    .expect("static semantic landmark selector");
    let mut blocks = Vec::new();
    let mut total_text_bytes = 0usize;
    for element in document.select(&landmarks) {
        if blocks.len() == MAX_DOM_BLOCKS_V1 {
            return Err(Error::TooManyDomBlocks);
        }
        let role = match element.value().name() {
            "nav" => BlockRole::Navigation,
            "header" => BlockRole::Header,
            "aside" => BlockRole::Sidebar,
            "footer" => BlockRole::Footer,
            "form" => BlockRole::Form,
            _ => BlockRole::ContentMain,
        };
        let node = document.tree.get(element.id()).ok_or(Error::NoVisibleContent)?;
        let text = collect_visible_text_with_root(node, true)?;
        if text.is_empty() {
            continue;
        }
        total_text_bytes = total_text_bytes
            .checked_add(text.len())
            .ok_or(Error::VisibleContentTooLarge)?;
        if total_text_bytes > MAX_VISIBLE_TEXT_BYTES_V1 {
            return Err(Error::VisibleContentTooLarge);
        }
        blocks.push(StrictDomBlockInput {
            dom_block_id: format!("raw-page-{}-block-{:03}", source.page_id, blocks.len() + 1),
            block_role: role,
            text,
        });
    }
    if !blocks.iter().any(|block| block.block_role == BlockRole::ContentMain) {
        let main_selector = Selector::parse("main").expect("static main selector");
        let body_selector = Selector::parse("body").expect("static body selector");
        let scope = document
            .select(&main_selector)
            .next()
            .or_else(|| document.select(&body_selector).next())
            .ok_or(Error::NoVisibleContent)?;
        let node = document.tree.get(scope.id()).ok_or(Error::NoVisibleContent)?;
        let text = collect_visible_text(node)?;
        if !text.is_empty() {
            if blocks.len() == MAX_DOM_BLOCKS_V1 {
                return Err(Error::TooManyDomBlocks);
            }
            total_text_bytes = total_text_bytes
                .checked_add(text.len())
                .ok_or(Error::VisibleContentTooLarge)?;
            if total_text_bytes > MAX_VISIBLE_TEXT_BYTES_V1 {
                return Err(Error::VisibleContentTooLarge);
            }
            blocks.push(StrictDomBlockInput {
                dom_block_id: format!("raw-page-{}-block-{:03}", source.page_id, blocks.len() + 1),
                block_role: BlockRole::ContentMain,
                text,
            });
        }
    }
    if blocks.is_empty() {
        return Err(Error::NoVisibleContent);
    }
    let input = DomBlockRelevanceInput { blocks };
    let payload = serde_json::to_vec(&input).map_err(|_| Error::WireSerializationFailed)?;
    if payload.len() > DAG_MAX_INPUT_BYTES {
        return Err(Error::WirePayloadTooLarge);
    }
    Ok(DerivedDomBlockRelevanceV1 {
        input,
        payload_hash: blake3_hex(&payload),
        payload,
        raw_content_hash,
    })
}

/// Deterministic source-to-wire transformation used only by an independently
/// authenticated Acquisition source issuer. Returns exact bytes/digests, not
/// a permit to run Temporal or trust an arbitrary caller's provenance.
pub fn classify_source_page_payload_from_pinned_html_v1(
    source: &CapturedHtmlPageV1,
) -> Result<DerivedClassifySourcePageV1, TrustedSourceTransformErrorV1> {
    let (input, raw_content_hash) = derive_classifier_input_v1(source)?;
    let payload = serde_json::to_vec(&input)
        .map_err(|_| TrustedSourceTransformErrorV1::WireSerializationFailed)?;
    if payload.len() > DAG_MAX_INPUT_BYTES {
        return Err(TrustedSourceTransformErrorV1::WirePayloadTooLarge);
    }
    let payload_hash = blake3_hex(&payload);
    Ok(DerivedClassifySourcePageV1 {
        input,
        payload,
        payload_hash,
        raw_content_hash,
    })
}

#[cfg(test)]
#[path = "trusted_source_transform_v1_tests.rs"]
mod tests;
