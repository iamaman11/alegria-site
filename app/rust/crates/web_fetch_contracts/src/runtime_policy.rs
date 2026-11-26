//! The owner's fail-closed egress policy, split into testable checkpoints.
//!
//! No call in this module performs I/O or mints a trusted owner receipt.
//! The Temporal owner must obtain operator policy independently from its
//! control plane, resolve DNS with a trusted resolver, pin the actual HTTPS
//! connection to the selected IP, verify TLS, fetch and interpret robots for
//! every target path, and enforce decoded-byte limits while streaming.
//! Supplying invented observations to these functions is not authentication.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;
use url::Url;

use crate::{
    checked_allowed_hosts, checked_url_host, validate_static_redirect_hop_v1,
    validate_static_web_fetch_request_v1, WebFetchPolicyError, WebFetchRequestV1,
    MAX_REDIRECTS_V1, MAX_RESPONSE_BYTES_V1, MAX_TIMEOUT_SECONDS_V1,
};

/// Independent operator policy; never deserialize this from a recipe or a
/// provider's response. Its owner must load it from trusted tenant admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorFetchPolicyV1 {
    pub authorized_tenant_id: String,
    pub allowed_hosts: BTreeSet<String>,
    pub max_response_bytes: u32,
    pub max_redirects: u8,
    pub timeout_seconds: u8,
}

/// A pre-connect decision. Transport must connect to `pinned_address`
/// without performing a second, independent hostname resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedWebFetchHopV1 {
    pub url: String,
    pub host: String,
    pub pinned_address: IpAddr,
    pub redirect_number: u8,
}

/// Server-side observations made by the independently implemented TLS and
/// robots transport. These are NOT untrusted recipe inputs or public APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchTransportProofV1 {
    pub connected_peer_address: IpAddr,
    pub tls_hostname_verified: bool,
    pub robots_path_allowed: bool,
    pub robots_policy_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchResponseProofV1 {
    pub status_code: u16,
    pub content_type: String,
    /// Count both raw wire bytes and decoded body bytes, including streaming
    /// decompression. A Content-Length header is never sufficient.
    pub received_bytes: u64,
    pub decoded_body_bytes: u64,
    pub total_elapsed_ms: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimeWebFetchError {
    #[error(transparent)]
    Static(#[from] WebFetchPolicyError),
    #[error("request tenant is not independently authorized")]
    TenantNotAdmitted,
    #[error("operator's own host allowlist forbids this hop")]
    OperatorHostDenied,
    #[error("request exceeds an independently configured operator budget")]
    OperatorBudgetExceeded,
    #[error("DNS has no independently usable public addresses")]
    MissingDnsAnswer,
    #[error("DNS answer contains a non-public, reserved, or embedded address")]
    UnsafeDnsAnswer,
    #[error("selected destination is not a pinned DNS answer")]
    UnpinnedAddress,
    #[error("TLS peer or connected address does not match authorized endpoint")]
    TransportNotVerified,
    #[error("robots policy must be proven and allow the requested path")]
    RobotsNotAllowed,
    #[error("HTTP result is not a successful HTML document")]
    NotHtmlSuccess,
    #[error("HTTP byte or elapsed-time limit exceeded")]
    RuntimeBudgetExceeded,
    #[error("redirect has no valid location or does not use a supported status")]
    InvalidRedirect,
    #[error("redirect loop detected")]
    RedirectLoop,
    #[error("trusted owner transport failed")]
    TransportFailed,
    #[error("transport did not supply a complete SHA-256/BLAKE3-sized content digest")]
    InvalidContentHash,
}

/// Opaque claim returned by the owner's bounded HTTP transport. The adapter
/// must enforce the *remaining* `byte_limit` *during* network read and
/// decompression, not just report counts after allocating an unbounded body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchTransportResponseV1 {
    pub connected_peer_address: IpAddr,
    pub tls_hostname_verified: bool,
    pub status_code: u16,
    pub content_type: String,
    pub received_bytes: u64,
    pub decoded_body_bytes: u64,
    /// Monotone elapsed time since the *first* DNS lookup, including all
    /// previous redirects, robots requests and HTTPS transfer attempts.
    pub total_elapsed_ms: u64,
    pub redirect_location: Option<String>,
    pub content_hash: Option<String>,
}

/// Injected by a separately admitted Temporal owner, never by recipe data.
/// Implementations MUST keep DNS/robots and HTTPS connections bound to the
/// selected IP, do certificate hostname verification, and refuse unsafe
/// redirects during robots retrieval as well. Neither this trait nor its
/// associated policy checks issue an operator admission or source receipt.
pub trait WebFetchOwnerTransportV1 {
    fn resolve_dns(&mut self, host: &str) -> Result<Vec<IpAddr>, RuntimeWebFetchError>;

    fn fetch_robots_for_pinned_hop(
        &mut self,
        hop: &PinnedWebFetchHopV1,
        remaining_byte_limit: u64,
        remaining_deadline_ms: u64,
    ) -> Result<WebFetchTransportProofV1, RuntimeWebFetchError>;

    fn fetch_html_with_limits(
        &mut self,
        hop: &PinnedWebFetchHopV1,
        remaining_byte_limit: u64,
        remaining_deadline_ms: u64,
    ) -> Result<WebFetchTransportResponseV1, RuntimeWebFetchError>;
}

/// A *transport-validated claim*, not a durable snapshot, source ownership
/// proof or Temporal History receipt. Owner must independently persist source
/// bytes and mint the typed receipt under a durable idempotency fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchTransportResultV1 {
    pub final_url: String,
    pub content_hash: String,
    pub validated_redirects: u8,
    pub decoded_body_bytes: u64,
    /// These owner-transport observations must be checked against durable
    /// bytes, then hashed/attested before minting WebPageSnapshotV1.
    pub visited_urls: Vec<String>,
    pub robots_hashes_by_url: BTreeMap<String, String>,
}

/// A conservative public Internet address check. Reject all private, loopback,
/// carrier-grade NAT, test, metadata, multicast, link-local and IPv4-embedded
/// IPv6 destinations, including mixed public/private DNS answer sets.
pub fn is_public_egress_ip_v1(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => public_ipv4(ip),
        IpAddr::V6(ip) => public_ipv6(ip),
    }
}

fn public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    match a {
        0 | 10 | 127 | 224..=255 => false,
        100 if (64..=127).contains(&b) => false, // shared address space
        169 if b == 254 => false,
        172 if (16..=31).contains(&b) => false,
        192 if b == 0 || b == 168 => false,
        192 if b == 88 && c == 99 => false, // deprecated relay
        198 if b == 18 || b == 19 || (b == 51 && c == 100) => false,
        203 if b == 0 && c == 113 => false,
        _ => true,
    }
}

fn public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    // Only unicast 2000::/3 can be an acceptable publicly routed IPv6
    // destination; exclude special-purpose reservations and tunneling.
    if segments[0] & 0xe000 != 0x2000 {
        return false;
    }
    if segments[0] == 0x2002 // 6to4, embeds arbitrary IPv4 destinations
        || (segments[0] == 0x2001 && segments[1] & 0xfe00 == 0) // 2001::/23
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
    {
        return false;
    }
    true
}

fn check_operator_admission(
    request: &WebFetchRequestV1,
    operator: &OperatorFetchPolicyV1,
) -> Result<BTreeSet<String>, RuntimeWebFetchError> {
    validate_static_web_fetch_request_v1(request)?;
    if request.tenant_id != operator.authorized_tenant_id {
        return Err(RuntimeWebFetchError::TenantNotAdmitted);
    }
    if operator.max_response_bytes == 0
        || operator.max_response_bytes > MAX_RESPONSE_BYTES_V1
        || operator.max_redirects > MAX_REDIRECTS_V1
        || operator.timeout_seconds == 0
        || operator.timeout_seconds > MAX_TIMEOUT_SECONDS_V1
        || request.max_response_bytes > operator.max_response_bytes
        || request.max_redirects > operator.max_redirects
        || request.timeout_seconds > operator.timeout_seconds
    {
        return Err(RuntimeWebFetchError::OperatorBudgetExceeded);
    }
    Ok(checked_allowed_hosts(&operator.allowed_hosts)?)
}

/// Call *before connecting* for the first URL and for EACH redirect. Resolve
/// target DNS afresh, reject entire unsafe answer sets, select one pinned IP,
/// then give this plan to the transport. A redirected origin never inherits
/// the first origin's DNS or robots decision.
pub fn prepare_runtime_web_fetch_hop_v1(
    request: &WebFetchRequestV1,
    operator: &OperatorFetchPolicyV1,
    url: &str,
    redirect_number: u8,
    dns_answers: &[IpAddr],
    selected_address: IpAddr,
) -> Result<PinnedWebFetchHopV1, RuntimeWebFetchError> {
    let operator_hosts = check_operator_admission(request, operator)?;
    if redirect_number == 0 {
        if url != request.source_url {
            return Err(RuntimeWebFetchError::Static(WebFetchPolicyError::InvalidUrl));
        }
    } else {
        validate_static_redirect_hop_v1(request, url, redirect_number - 1)?;
    }
    let (_, host) = checked_url_host(url)?;
    if !operator_hosts.contains(&host) {
        return Err(RuntimeWebFetchError::OperatorHostDenied);
    }
    if dns_answers.is_empty() || dns_answers.len() > 16 {
        return Err(RuntimeWebFetchError::MissingDnsAnswer);
    }
    if dns_answers
        .iter()
        .any(|address| !is_public_egress_ip_v1(*address))
    {
        return Err(RuntimeWebFetchError::UnsafeDnsAnswer);
    }
    if !dns_answers.contains(&selected_address) {
        return Err(RuntimeWebFetchError::UnpinnedAddress);
    }
    Ok(PinnedWebFetchHopV1 {
        url: url.to_owned(),
        host,
        pinned_address: selected_address,
        redirect_number,
    })
}

/// Gate an HTTP request only after the actual HTTPS transport reports its
/// pinned peer, verified certificate hostname and fresh per-path robots proof.
pub fn authorize_runtime_web_fetch_http_v1(
    hop: &PinnedWebFetchHopV1,
    transport: &WebFetchTransportProofV1,
) -> Result<(), RuntimeWebFetchError> {
    if !transport.tls_hostname_verified || transport.connected_peer_address != hop.pinned_address {
        return Err(RuntimeWebFetchError::TransportNotVerified);
    }
    if !transport.robots_path_allowed
        || transport.robots_policy_hash.len() != 64
        || !transport
            .robots_policy_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RuntimeWebFetchError::RobotsNotAllowed);
    }
    Ok(())
}

/// Gate persistence of an HTML snapshot. The transport must apply this byte
/// budget incrementally while streaming, too, so an infinite response cannot
/// exhaust memory before the final check.
pub fn authorize_runtime_web_fetch_response_v1(
    request: &WebFetchRequestV1,
    operator: &OperatorFetchPolicyV1,
    response: &WebFetchResponseProofV1,
) -> Result<(), RuntimeWebFetchError> {
    check_operator_admission(request, operator)?;
    let limit = u64::from(request.max_response_bytes);
    if response.received_bytes > limit
        || response.decoded_body_bytes > limit
        || response.total_elapsed_ms > u64::from(request.timeout_seconds) * 1000
    {
        return Err(RuntimeWebFetchError::RuntimeBudgetExceeded);
    }
    let mime = response
        .content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !(200..=299).contains(&response.status_code)
        || (mime != "text/html" && mime != "application/xhtml+xml")
    {
        return Err(RuntimeWebFetchError::NotHtmlSuccess);
    }
    Ok(())
}

/// The complete policy-controlled adapter loop exercised by a real owner or
/// a deterministic fake. No redirect reaches DNS or a socket before static
/// and independently configured host checks. A trusted transport is still
/// mandatory: this function cannot attest its network implementation.
pub fn drive_owner_web_fetch_v1<T: WebFetchOwnerTransportV1>(
    request: &WebFetchRequestV1,
    operator: &OperatorFetchPolicyV1,
    transport: &mut T,
) -> Result<WebFetchTransportResultV1, RuntimeWebFetchError> {
    let operator_hosts = check_operator_admission(request, operator)?;
    let mut target = request.source_url.clone();
    let mut visited = BTreeSet::new();
    let mut visited_in_order = Vec::new();
    let mut redirect_number = 0u8;
    let mut robots_hashes_by_url = BTreeMap::new();
    let max_bytes = u64::from(request.max_response_bytes);
    let deadline_ms = u64::from(request.timeout_seconds) * 1000;
    let mut aggregate_wire_bytes = 0u64;
    let mut aggregate_decoded_bytes = 0u64;
    let mut previous_elapsed_ms = 0u64;

    loop {
        let (_, host) = checked_url_host(&target)?;
        if redirect_number == 0 {
            if target != request.source_url {
                return Err(RuntimeWebFetchError::Static(WebFetchPolicyError::InvalidUrl));
            }
        } else {
            validate_static_redirect_hop_v1(request, &target, redirect_number - 1)?;
        }
        if !operator_hosts.contains(&host) {
            return Err(RuntimeWebFetchError::OperatorHostDenied);
        }
        if !visited.insert(target.clone()) {
            return Err(RuntimeWebFetchError::RedirectLoop);
        }
        visited_in_order.push(target.clone());
        // The owner resolver must return all answers, not silently omit a
        // private alternative from a mixed DNS response.
        let answers = transport.resolve_dns(&host)?;
        let selected = *answers
            .first()
            .ok_or(RuntimeWebFetchError::MissingDnsAnswer)?;
        let hop = prepare_runtime_web_fetch_hop_v1(
            request,
            operator,
            &target,
            redirect_number,
            &answers,
            selected,
        )?;
        let remaining_bytes = max_bytes
            .saturating_sub(aggregate_wire_bytes.max(aggregate_decoded_bytes));
        let remaining_ms = deadline_ms.saturating_sub(previous_elapsed_ms);
        if remaining_bytes == 0 || remaining_ms == 0 {
            return Err(RuntimeWebFetchError::RuntimeBudgetExceeded);
        }
        let robots = transport.fetch_robots_for_pinned_hop(
            &hop,
            remaining_bytes.min(256 * 1024),
            remaining_ms,
        )?;
        authorize_runtime_web_fetch_http_v1(&hop, &robots)?;
        robots_hashes_by_url.insert(target.clone(), robots.robots_policy_hash);
        let response = transport.fetch_html_with_limits(&hop, remaining_bytes, remaining_ms)?;
        if !response.tls_hostname_verified || response.connected_peer_address != selected {
            return Err(RuntimeWebFetchError::TransportNotVerified);
        }
        aggregate_wire_bytes = aggregate_wire_bytes
            .checked_add(response.received_bytes)
            .ok_or(RuntimeWebFetchError::RuntimeBudgetExceeded)?;
        aggregate_decoded_bytes = aggregate_decoded_bytes
            .checked_add(response.decoded_body_bytes)
            .ok_or(RuntimeWebFetchError::RuntimeBudgetExceeded)?;
        if aggregate_wire_bytes > max_bytes
            || aggregate_decoded_bytes > max_bytes
            || response.total_elapsed_ms > deadline_ms
            || response.total_elapsed_ms < previous_elapsed_ms
        {
            return Err(RuntimeWebFetchError::RuntimeBudgetExceeded);
        }
        previous_elapsed_ms = response.total_elapsed_ms;

        match response.status_code {
            301 | 302 | 303 | 307 | 308 => {
                let location = response
                    .redirect_location
                    .ok_or(RuntimeWebFetchError::InvalidRedirect)?;
                let destination = Url::parse(&target)
                    .map_err(|_| RuntimeWebFetchError::InvalidRedirect)?
                    .join(&location)
                    .map_err(|_| RuntimeWebFetchError::InvalidRedirect)?;
                let next = destination.as_str().to_owned();
                // Increment before DNS; at max-redirect count this check
                // refuses the next URL and makes no external request.
                validate_static_redirect_hop_v1(request, &next, redirect_number)?;
                redirect_number += 1;
                target = next;
            }
            200..=299 => {
                authorize_runtime_web_fetch_response_v1(
                    request,
                    operator,
                    &WebFetchResponseProofV1 {
                        status_code: response.status_code,
                        content_type: response.content_type,
                        received_bytes: response.received_bytes,
                        decoded_body_bytes: response.decoded_body_bytes,
                        total_elapsed_ms: response.total_elapsed_ms,
                    },
                )?;
                let content_hash = response
                    .content_hash
                    .filter(|hash| {
                        hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                    .ok_or(RuntimeWebFetchError::InvalidContentHash)?;
                return Ok(WebFetchTransportResultV1 {
                    final_url: target,
                    content_hash,
                    validated_redirects: redirect_number,
                    decoded_body_bytes: response.decoded_body_bytes,
                    visited_urls: visited_in_order,
                    robots_hashes_by_url,
                });
            }
            _ => return Err(RuntimeWebFetchError::NotHtmlSuccess),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn request() -> WebFetchRequestV1 {
        WebFetchRequestV1 {
            tenant_id: "tenant-a".into(),
            source_url: "https://www.example.com/review".into(),
            allowed_hosts: BTreeSet::from([
                "www.example.com".into(),
                "pages.example.net".into(),
            ]),
            max_response_bytes: 64 * 1024,
            max_redirects: 2,
            timeout_seconds: 10,
        }
    }

    fn operator() -> OperatorFetchPolicyV1 {
        OperatorFetchPolicyV1 {
            authorized_tenant_id: "tenant-a".into(),
            allowed_hosts: BTreeSet::from([
                "www.example.com".into(),
                "pages.example.net".into(),
            ]),
            max_response_bytes: 128 * 1024,
            max_redirects: 3,
            timeout_seconds: 15,
        }
    }

    fn public_ip() -> IpAddr {
        "1.1.1.1".parse().unwrap()
    }

    fn plan() -> PinnedWebFetchHopV1 {
        prepare_runtime_web_fetch_hop_v1(
            &request(),
            &operator(),
            &request().source_url,
            0,
            &[public_ip()],
            public_ip(),
        )
        .unwrap()
    }

    #[test]
    fn public_egress_policy_rejects_private_metadata_and_special_ip_ranges() {
        for address in [
            "127.0.0.1",
            "10.0.1.2",
            "100.64.1.5",
            "169.254.169.254",
            "172.16.0.4",
            "192.168.1.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.12",
            "224.1.1.1",
            "255.255.255.255",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:c0a8:101::1",
            "2001:0:c0a8:101::1",
            "64:ff9b::c0a8:101",
        ] {
            assert!(!is_public_egress_ip_v1(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "9.9.9.9", "2606:4700:4700::1111"] {
            assert!(is_public_egress_ip_v1(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn operator_policy_cannot_be_expanded_by_recipe() {
        let mut policy = operator();
        policy.authorized_tenant_id = "another-tenant".into();
        assert_eq!(
            prepare_runtime_web_fetch_hop_v1(
                &request(),
                &policy,
                &request().source_url,
                0,
                &[public_ip()],
                public_ip()
            )
            .unwrap_err(),
            RuntimeWebFetchError::TenantNotAdmitted
        );
        let mut policy = operator();
        policy.allowed_hosts = BTreeSet::from(["pages.example.net".into()]);
        assert_eq!(
            prepare_runtime_web_fetch_hop_v1(
                &request(),
                &policy,
                &request().source_url,
                0,
                &[public_ip()],
                public_ip()
            )
            .unwrap_err(),
            RuntimeWebFetchError::OperatorHostDenied
        );
        let mut policy = operator();
        policy.max_response_bytes = 1024;
        assert_eq!(
            prepare_runtime_web_fetch_hop_v1(
                &request(),
                &policy,
                &request().source_url,
                0,
                &[public_ip()],
                public_ip()
            )
            .unwrap_err(),
            RuntimeWebFetchError::OperatorBudgetExceeded
        );
    }

    #[test]
    fn dns_answer_mixing_and_unpinned_peer_are_rejected() {
        assert_eq!(
            prepare_runtime_web_fetch_hop_v1(
                &request(),
                &operator(),
                &request().source_url,
                0,
                &[public_ip(), "10.0.0.1".parse().unwrap()],
                public_ip()
            )
            .unwrap_err(),
            RuntimeWebFetchError::UnsafeDnsAnswer
        );
        assert_eq!(
            prepare_runtime_web_fetch_hop_v1(
                &request(),
                &operator(),
                &request().source_url,
                0,
                &[public_ip()],
                "8.8.8.8".parse().unwrap()
            )
            .unwrap_err(),
            RuntimeWebFetchError::UnpinnedAddress
        );
        let mut peer = WebFetchTransportProofV1 {
            connected_peer_address: "8.8.8.8".parse().unwrap(),
            tls_hostname_verified: true,
            robots_path_allowed: true,
            robots_policy_hash: "a".repeat(64),
        };
        assert_eq!(
            authorize_runtime_web_fetch_http_v1(&plan(), &peer).unwrap_err(),
            RuntimeWebFetchError::TransportNotVerified
        );
        peer.connected_peer_address = public_ip();
        peer.tls_hostname_verified = false;
        assert_eq!(
            authorize_runtime_web_fetch_http_v1(&plan(), &peer).unwrap_err(),
            RuntimeWebFetchError::TransportNotVerified
        );
    }

    #[test]
    fn each_redirect_rechecks_host_dns_tls_and_robots() {
        let redirect = "https://pages.example.net/feedback";
        let first_hop = prepare_runtime_web_fetch_hop_v1(
            &request(),
            &operator(),
            redirect,
            1,
            &[public_ip()],
            public_ip(),
        )
        .unwrap();
        assert_eq!(first_hop.host, "pages.example.net");
        let second_hop = prepare_runtime_web_fetch_hop_v1(
            &request(),
            &operator(),
            redirect,
            2,
            &[public_ip()],
            public_ip(),
        )
        .unwrap();
        assert_eq!(second_hop.redirect_number, 2);
        assert!(prepare_runtime_web_fetch_hop_v1(
            &request(),
            &operator(),
            redirect,
            3,
            &[public_ip()],
            public_ip()
        )
        .is_err());
        let mut denied = WebFetchTransportProofV1 {
            connected_peer_address: public_ip(),
            tls_hostname_verified: true,
            robots_path_allowed: false,
            robots_policy_hash: "a".repeat(64),
        };
        assert_eq!(
            authorize_runtime_web_fetch_http_v1(&first_hop, &denied).unwrap_err(),
            RuntimeWebFetchError::RobotsNotAllowed
        );
        denied.robots_path_allowed = true;
        assert!(authorize_runtime_web_fetch_http_v1(&first_hop, &denied).is_ok());
        assert!(prepare_runtime_web_fetch_hop_v1(
            &request(),
            &operator(),
            "https://pages.example.net.evil.org/",
            1,
            &[public_ip()],
            public_ip()
        )
        .is_err());
    }

    #[test]
    fn decoded_bytes_mime_and_deadline_gate_snapshot_persistence() {
        let valid = WebFetchResponseProofV1 {
            status_code: 200,
            content_type: "text/html; charset=utf-8".into(),
            received_bytes: 400,
            decoded_body_bytes: 512,
            total_elapsed_ms: 9_999,
        };
        assert!(authorize_runtime_web_fetch_response_v1(&request(), &operator(), &valid).is_ok());
        let oversized = WebFetchResponseProofV1 {
            decoded_body_bytes: 65_537,
            ..valid.clone()
        };
        assert_eq!(
            authorize_runtime_web_fetch_response_v1(&request(), &operator(), &oversized)
                .unwrap_err(),
            RuntimeWebFetchError::RuntimeBudgetExceeded
        );
        let expired = WebFetchResponseProofV1 {
            total_elapsed_ms: 10_001,
            ..valid.clone()
        };
        assert!(authorize_runtime_web_fetch_response_v1(&request(), &operator(), &expired).is_err());
        let pdf = WebFetchResponseProofV1 {
            content_type: "application/pdf".into(),
            ..valid
        };
        assert_eq!(
            authorize_runtime_web_fetch_response_v1(&request(), &operator(), &pdf).unwrap_err(),
            RuntimeWebFetchError::NotHtmlSuccess
        );
    }

    struct MockOwnerTransport {
        dns: BTreeMap<String, Vec<IpAddr>>,
        answers: VecDeque<WebFetchTransportResponseV1>,
        robots_denied_host: Option<String>,
        resolved: Vec<String>,
        robots: Vec<String>,
        fetched: Vec<String>,
        limits: Vec<(u64, u64)>,
    }

    impl MockOwnerTransport {
        fn new(answers: Vec<WebFetchTransportResponseV1>) -> Self {
            Self {
                dns: BTreeMap::from([
                    ("www.example.com".into(), vec![public_ip()]),
                    ("pages.example.net".into(), vec!["8.8.8.8".parse().unwrap()]),
                ]),
                answers: answers.into(),
                robots_denied_host: None,
                resolved: Vec::new(),
                robots: Vec::new(),
                fetched: Vec::new(),
                limits: Vec::new(),
            }
        }
    }

    impl WebFetchOwnerTransportV1 for MockOwnerTransport {
        fn resolve_dns(&mut self, host: &str) -> Result<Vec<IpAddr>, RuntimeWebFetchError> {
            self.resolved.push(host.into());
            self.dns
                .get(host)
                .cloned()
                .ok_or(RuntimeWebFetchError::TransportFailed)
        }

        fn fetch_robots_for_pinned_hop(
            &mut self,
            hop: &PinnedWebFetchHopV1,
            remaining_byte_limit: u64,
            remaining_deadline_ms: u64,
        ) -> Result<WebFetchTransportProofV1, RuntimeWebFetchError> {
            assert!(remaining_byte_limit <= u64::from(request().max_response_bytes));
            assert!(remaining_deadline_ms <= u64::from(request().timeout_seconds) * 1000);
            self.robots.push(hop.url.clone());
            Ok(WebFetchTransportProofV1 {
                connected_peer_address: hop.pinned_address,
                tls_hostname_verified: true,
                robots_path_allowed: self.robots_denied_host.as_ref() != Some(&hop.host),
                robots_policy_hash: "d".repeat(64),
            })
        }

        fn fetch_html_with_limits(
            &mut self,
            hop: &PinnedWebFetchHopV1,
            remaining_byte_limit: u64,
            remaining_deadline_ms: u64,
        ) -> Result<WebFetchTransportResponseV1, RuntimeWebFetchError> {
            self.limits
                .push((remaining_byte_limit, remaining_deadline_ms));
            self.fetched.push(hop.url.clone());
            self.answers
                .pop_front()
                .ok_or(RuntimeWebFetchError::TransportFailed)
        }
    }

    fn redirect_response() -> WebFetchTransportResponseV1 {
        WebFetchTransportResponseV1 {
            connected_peer_address: public_ip(),
            tls_hostname_verified: true,
            status_code: 302,
            content_type: String::new(),
            received_bytes: 128,
            decoded_body_bytes: 0,
            total_elapsed_ms: 1000,
            redirect_location: Some("https://pages.example.net/feedback".into()),
            content_hash: None,
        }
    }

    fn final_response() -> WebFetchTransportResponseV1 {
        WebFetchTransportResponseV1 {
            connected_peer_address: "8.8.8.8".parse().unwrap(),
            tls_hostname_verified: true,
            status_code: 200,
            content_type: "text/html".into(),
            received_bytes: 200,
            decoded_body_bytes: 300,
            total_elapsed_ms: 2000,
            redirect_location: None,
            content_hash: Some("e".repeat(64)),
        }
    }

    #[test]
    fn complete_owner_loop_rechecks_dns_and_robots_per_redirect() {
        let mut transport = MockOwnerTransport::new(vec![redirect_response(), final_response()]);
        let result = drive_owner_web_fetch_v1(&request(), &operator(), &mut transport).unwrap();
        assert_eq!(result.final_url, "https://pages.example.net/feedback");
        assert_eq!(result.validated_redirects, 1);
        assert_eq!(result.decoded_body_bytes, 300);
        assert_eq!(
            result.visited_urls,
            vec![
                "https://www.example.com/review",
                "https://pages.example.net/feedback"
            ]
        );
        assert_eq!(result.robots_hashes_by_url.len(), 2);
        assert_eq!(transport.resolved, vec!["www.example.com", "pages.example.net"]);
        assert_eq!(transport.robots.len(), 2);
        assert_eq!(transport.fetched.len(), 2);
        assert_eq!(transport.limits[0], (65_536, 10_000));
        assert_eq!(transport.limits[1], (65_408, 9_000));
    }

    #[test]
    fn private_dns_on_redirect_stops_before_robots_and_http() {
        let mut transport = MockOwnerTransport::new(vec![redirect_response(), final_response()]);
        transport.dns.insert(
            "pages.example.net".into(),
            vec!["8.8.8.8".parse().unwrap(), "169.254.169.254".parse().unwrap()],
        );
        let error = drive_owner_web_fetch_v1(&request(), &operator(), &mut transport).unwrap_err();
        assert_eq!(error, RuntimeWebFetchError::UnsafeDnsAnswer);
        assert_eq!(transport.resolved.len(), 2);
        assert_eq!(transport.robots.len(), 1);
        assert_eq!(transport.fetched.len(), 1);
    }

    #[test]
    fn robots_denied_on_redirect_stops_before_second_http_get() {
        let mut transport = MockOwnerTransport::new(vec![redirect_response(), final_response()]);
        transport.robots_denied_host = Some("pages.example.net".into());
        let error = drive_owner_web_fetch_v1(&request(), &operator(), &mut transport).unwrap_err();
        assert_eq!(error, RuntimeWebFetchError::RobotsNotAllowed);
        assert_eq!(transport.robots.len(), 2);
        assert_eq!(transport.fetched.len(), 1);
    }

    #[test]
    fn rebinding_between_dns_and_second_https_response_is_rejected() {
        let mut second = final_response();
        second.connected_peer_address = "127.0.0.1".parse().unwrap();
        let mut transport = MockOwnerTransport::new(vec![redirect_response(), second]);
        assert_eq!(
            drive_owner_web_fetch_v1(&request(), &operator(), &mut transport).unwrap_err(),
            RuntimeWebFetchError::TransportNotVerified
        );
        assert_eq!(transport.fetched.len(), 2);
    }

    #[test]
    fn unsafe_redirect_never_resolves_target_host() {
        let mut unsafe_response = redirect_response();
        unsafe_response.redirect_location = Some("http://169.254.169.254/latest".into());
        let mut transport = MockOwnerTransport::new(vec![unsafe_response]);
        assert!(drive_owner_web_fetch_v1(&request(), &operator(), &mut transport).is_err());
        assert_eq!(transport.resolved.len(), 1);
        assert_eq!(transport.robots.len(), 1);
        assert_eq!(transport.fetched.len(), 1);
    }

    #[test]
    fn aggregated_redirect_bytes_and_loops_fail_closed() {
        let mut first = redirect_response();
        first.received_bytes = 40_000;
        let mut last = final_response();
        last.received_bytes = 30_000;
        let mut transport = MockOwnerTransport::new(vec![first, last]);
        assert_eq!(
            drive_owner_web_fetch_v1(&request(), &operator(), &mut transport).unwrap_err(),
            RuntimeWebFetchError::RuntimeBudgetExceeded
        );

        let mut loop_response = redirect_response();
        loop_response.redirect_location = Some("/review".into());
        let mut transport = MockOwnerTransport::new(vec![loop_response]);
        assert_eq!(
            drive_owner_web_fetch_v1(&request(), &operator(), &mut transport).unwrap_err(),
            RuntimeWebFetchError::RedirectLoop
        );
        assert_eq!(transport.resolved.len(), 1);
        assert_eq!(transport.fetched.len(), 1);

        let mut slow_redirect = redirect_response();
        slow_redirect.total_elapsed_ms = 9_500;
        let mut slow_html = final_response();
        slow_html.total_elapsed_ms = 10_100;
        let mut transport = MockOwnerTransport::new(vec![slow_redirect, slow_html]);
        assert_eq!(
            drive_owner_web_fetch_v1(&request(), &operator(), &mut transport).unwrap_err(),
            RuntimeWebFetchError::RuntimeBudgetExceeded
        );
        assert_eq!(transport.limits[1].1, 500);
    }
}
