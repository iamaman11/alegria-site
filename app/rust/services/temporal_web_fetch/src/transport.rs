//! Real public-Internet HTTPS transport, never a mock provider response.
//!
//! Every hop is resolved afresh. A new no-proxy, no-redirect, hostname-TLS-
//! verified Client pins the selected public address; the actual peer IP is
//! checked before reading bytes. Robots is fetched through that same guard.
//! Body reads are bounded *while streaming* and compression is refused so
//! decoded HTML can never exceed the accounting limit invisibly.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, LOCATION, USER_AGENT,
};
use serde::{Deserialize, Serialize};
use url::Url;
use web_fetch_contracts::runtime_policy::{
    drive_owner_web_fetch_v1, is_public_egress_ip_v1, OperatorFetchPolicyV1,
    PinnedWebFetchHopV1, RuntimeWebFetchError, WebFetchOwnerTransportV1,
    WebFetchTransportProofV1, WebFetchTransportResponseV1, WebFetchTransportResultV1,
};
use web_fetch_contracts::WebFetchRequestV1;

const BOT_AGENT: &str = "siteArchitectBot";
const HTTP_AGENT: &str = "siteArchitectBot/1.0 (+public-page-fetch)";
const MAX_ROBOTS_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
pub struct NativePageCaptureV1 {
    pub checked_transport: WebFetchTransportResultV1,
    /// The exact verified HTML bytes. Not a durable or operator-issued receipt.
    pub html_bytes: Vec<u8>,
    /// Raw proof retained for independently verified immutable persistence.
    pub robots_by_url: BTreeMap<String, RobotsResponseEvidenceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotsResponseEvidenceV1 {
    pub status_code: u16,
    pub body: Vec<u8>,
}

impl RobotsResponseEvidenceV1 {
    pub fn digest(&self) -> String {
        if self.status_code == 404 {
            blake3::hash(b"robots.txt:404").to_hex().to_string()
        } else {
            blake3::hash(&self.body).to_hex().to_string()
        }
    }
}

/// Run only after a separate admission owner has issued this exact tenant's
/// operator policy. The result must be durably stored and checked against the
/// bytes before a Temporal Child can emit a trusted `WebPageSnapshotV1`.
pub fn capture_admitted_public_page_v1(
    request: &WebFetchRequestV1,
    operator: &OperatorFetchPolicyV1,
) -> Result<NativePageCaptureV1, RuntimeWebFetchError> {
    let mut native = NativeHttpsTransportV1::new(Duration::from_secs(u64::from(
        request.timeout_seconds,
    )));
    let checked_transport = drive_owner_web_fetch_v1(request, operator, &mut native)?;
    let html_bytes = native
        .final_html_bytes
        .take()
        .ok_or(RuntimeWebFetchError::TransportFailed)?;
    if blake3::hash(&html_bytes).to_hex().to_string() != checked_transport.content_hash
        || html_bytes.len() as u64 != checked_transport.decoded_body_bytes
    {
        return Err(RuntimeWebFetchError::InvalidContentHash);
    }
    if native.robots_by_url.len() != checked_transport.robots_hashes_by_url.len()
        || checked_transport.robots_hashes_by_url.iter().any(|(url, hash)| {
            native.robots_by_url.get(url).is_none_or(|observation| observation.digest() != *hash)
        })
    {
        return Err(RuntimeWebFetchError::RobotsNotAllowed);
    }
    Ok(NativePageCaptureV1 {
        checked_transport,
        html_bytes,
        robots_by_url: native.robots_by_url,
    })
}

pub struct NativeHttpsTransportV1 {
    started: Instant,
    total_timeout: Duration,
    total_robots_bytes: u64,
    final_html_bytes: Option<Vec<u8>>,
    robots_by_url: BTreeMap<String, RobotsResponseEvidenceV1>,
}

impl NativeHttpsTransportV1 {
    pub fn new(total_timeout: Duration) -> Self {
        Self {
            started: Instant::now(),
            total_timeout,
            total_robots_bytes: 0,
            final_html_bytes: None,
            robots_by_url: BTreeMap::new(),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn time_left(&self, remaining_ms: u64) -> Result<Duration, RuntimeWebFetchError> {
        let until_total = self
            .total_timeout
            .checked_sub(self.started.elapsed())
            .ok_or(RuntimeWebFetchError::RuntimeBudgetExceeded)?;
        if remaining_ms == 0 || until_total.is_zero() {
            return Err(RuntimeWebFetchError::RuntimeBudgetExceeded);
        }
        Ok(until_total.min(Duration::from_millis(remaining_ms)))
    }

    fn client_for(
        &self,
        hop: &PinnedWebFetchHopV1,
        timeout: Duration,
    ) -> Result<Client, RuntimeWebFetchError> {
        // In particular, NEVER use an HTTPS proxy; its remote_addr would not
        // represent the intended source and could bypass DNS/peer admission.
        Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .resolve(&hop.host, SocketAddr::new(hop.pinned_address, 443))
            .timeout(timeout)
            .build()
            .map_err(|_| RuntimeWebFetchError::TransportFailed)
    }

    fn verified_response(
        &self,
        response: &Response,
        hop: &PinnedWebFetchHopV1,
    ) -> Result<(), RuntimeWebFetchError> {
        if response.remote_addr().map(|addr| addr.ip()) != Some(hop.pinned_address)
            || response.url().scheme() != "https"
            || response.url().host_str() != Some(hop.host.as_str())
        {
            return Err(RuntimeWebFetchError::TransportNotVerified);
        }
        Ok(())
    }

    fn read_bounded(
        &self,
        mut response: Response,
        budget: u64,
    ) -> Result<Vec<u8>, RuntimeWebFetchError> {
        if response.content_length().is_some_and(|len| len > budget) {
            return Err(RuntimeWebFetchError::RuntimeBudgetExceeded);
        }
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|v| v.as_bytes() != b"identity")
        {
            return Err(RuntimeWebFetchError::TransportFailed);
        }
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(budget.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| RuntimeWebFetchError::TransportFailed)?;
        if bytes.len() as u64 > budget {
            return Err(RuntimeWebFetchError::RuntimeBudgetExceeded);
        }
        Ok(bytes)
    }
}

impl WebFetchOwnerTransportV1 for NativeHttpsTransportV1 {
    fn resolve_dns(&mut self, host: &str) -> Result<Vec<IpAddr>, RuntimeWebFetchError> {
        // Refuse a host whose DNS answer contains *any* unsafe IP, even when
        // it also returns a valid public address. Caller re-checks the set.
        // libc resolver calls can otherwise outlive the entire HTTP budget.
        // At most six short-lived lookups are started (initial + redirects).
        let resolver_host = host.to_owned();
        let (send, receive) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = (resolver_host.as_str(), 443)
                .to_socket_addrs()
                .map(|addrs| addrs.map(|addr| addr.ip()).collect::<BTreeSet<IpAddr>>());
            let _ = send.send(result);
        });
        let answers = receive
            .recv_timeout(self.time_left(u64::MAX)?)
            .map_err(|_| RuntimeWebFetchError::RuntimeBudgetExceeded)?
            .map_err(|_| RuntimeWebFetchError::MissingDnsAnswer)?;
        if answers.is_empty() {
            return Err(RuntimeWebFetchError::MissingDnsAnswer);
        }
        if answers.iter().any(|ip| !is_public_egress_ip_v1(*ip)) {
            return Err(RuntimeWebFetchError::UnsafeDnsAnswer);
        }
        if self.started.elapsed() >= self.total_timeout {
            return Err(RuntimeWebFetchError::RuntimeBudgetExceeded);
        }
        Ok(answers.into_iter().collect())
    }

    fn fetch_robots_for_pinned_hop(
        &mut self,
        hop: &PinnedWebFetchHopV1,
        remaining_byte_limit: u64,
        remaining_deadline_ms: u64,
    ) -> Result<WebFetchTransportProofV1, RuntimeWebFetchError> {
        let remaining_robots = MAX_ROBOTS_BYTES.saturating_sub(self.total_robots_bytes);
        let limit = remaining_byte_limit.min(remaining_robots);
        if limit == 0 {
            return Err(RuntimeWebFetchError::RuntimeBudgetExceeded);
        }
        let mut robots_url = Url::parse(&hop.url)
            .map_err(|_| RuntimeWebFetchError::TransportFailed)?;
        robots_url.set_path("/robots.txt");
        robots_url.set_query(None);
        let client = self.client_for(hop, self.time_left(remaining_deadline_ms)?)?;
        let response = client
            .get(robots_url)
            .header(USER_AGENT, HTTP_AGENT)
            .header(ACCEPT, "text/plain")
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .map_err(|_| RuntimeWebFetchError::TransportFailed)?;
        self.verified_response(&response, hop)?;
        let status = response.status();
        // robots.txt 404 has the protocol meaning "no robots rules".
        // In particular 401/403, 429, redirects, and 5xx are NOT treated as
        // permission to crawl or followed without an independent hop check.
        let (allowed, evidence) = if status == reqwest::StatusCode::NOT_FOUND {
            (true, RobotsResponseEvidenceV1 { status_code: 404, body: Vec::new() })
        } else if status.is_success() {
            let bytes = self.read_bounded(response, limit)?;
            let raw_text = std::str::from_utf8(&bytes)
                .map_err(|_| RuntimeWebFetchError::RobotsNotAllowed)?;
            let path_url = Url::parse(&hop.url)
                .map_err(|_| RuntimeWebFetchError::TransportFailed)?;
            let mut request_target = path_url.path().to_owned();
            if let Some(query) = path_url.query() {
                request_target.push('?');
                request_target.push_str(query);
            }
            (
                robots_allows_v1(raw_text, BOT_AGENT, &request_target)?,
                RobotsResponseEvidenceV1 { status_code: status.as_u16(), body: bytes },
            )
        } else {
            return Err(RuntimeWebFetchError::RobotsNotAllowed);
        };
        self.total_robots_bytes += evidence.body.len() as u64;
        let hash = evidence.digest();
        self.robots_by_url.insert(hop.url.clone(), evidence);
        Ok(WebFetchTransportProofV1 {
            connected_peer_address: hop.pinned_address,
            tls_hostname_verified: true,
            robots_path_allowed: allowed,
            robots_policy_hash: hash,
        })
    }

    fn fetch_html_with_limits(
        &mut self,
        hop: &PinnedWebFetchHopV1,
        remaining_byte_limit: u64,
        remaining_deadline_ms: u64,
    ) -> Result<WebFetchTransportResponseV1, RuntimeWebFetchError> {
        self.final_html_bytes = None;
        let client = self.client_for(hop, self.time_left(remaining_deadline_ms)?)?;
        let response = client
            .get(&hop.url)
            .header(USER_AGENT, HTTP_AGENT)
            .header(ACCEPT, "text/html,application/xhtml+xml")
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .map_err(|_| RuntimeWebFetchError::TransportFailed)?;
        self.verified_response(&response, hop)?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let redirect_location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        // A redirect's Location is validated by drive_owner_web_fetch_v1
        // *before* the redirected target is resolved or connected.
        let bytes = if (200..=299).contains(&status) {
            self.read_bounded(response, remaining_byte_limit)?
        } else {
            Vec::new()
        };
        let received_bytes = bytes.len() as u64;
        let content_hash = if (200..=299).contains(&status) {
            self.final_html_bytes = Some(bytes);
            self.final_html_bytes
                .as_ref()
                .map(|value| blake3::hash(value).to_hex().to_string())
        } else {
            None
        };
        Ok(WebFetchTransportResponseV1 {
            connected_peer_address: hop.pinned_address,
            tls_hostname_verified: true,
            status_code: status,
            content_type,
            received_bytes,
            decoded_body_bytes: received_bytes,
            total_elapsed_ms: self.elapsed_ms(),
            redirect_location,
            content_hash,
        })
    }
}

/// Minimal conservative REP parser. Unknown directives do not grant access;
/// invalid robots text, malformed group syntax and exotic pattern escapes
/// fail closed. Specific agent groups never inherit looser wildcard rules.
pub fn robots_allows_v1(
    raw: &str,
    user_agent: &str,
    request_target: &str,
) -> Result<bool, RuntimeWebFetchError> {
    if raw.contains('\0')
        || !request_target.starts_with('/')
        || raw.len() > MAX_ROBOTS_BYTES as usize
    {
        return Err(RuntimeWebFetchError::RobotsNotAllowed);
    }
    let requested_agent = user_agent.to_ascii_lowercase();
    let mut group_agents = Vec::<String>::new();
    let mut current_rules = Vec::<(bool, String)>::new();
    let mut grouped_rules = Vec::<(Vec<String>, Vec<(bool, String)>)>::new();
    let mut in_rules = false;

    fn finish(
        groups: &mut Vec<(Vec<String>, Vec<(bool, String)>)>,
        agents: &mut Vec<String>,
        rules: &mut Vec<(bool, String)>,
        in_rules: &mut bool,
    ) {
        if !agents.is_empty() {
            groups.push((std::mem::take(agents), std::mem::take(rules)));
        }
        *in_rules = false;
    }

    for raw_line in raw.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(RuntimeWebFetchError::RobotsNotAllowed);
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "user-agent" => {
                if in_rules {
                    finish(
                        &mut grouped_rules,
                        &mut group_agents,
                        &mut current_rules,
                        &mut in_rules,
                    );
                }
                if value.is_empty() || !value.is_ascii() || value.chars().any(char::is_whitespace) {
                    return Err(RuntimeWebFetchError::RobotsNotAllowed);
                }
                group_agents.push(value.to_ascii_lowercase());
            }
            "allow" | "disallow" if !group_agents.is_empty() => {
                in_rules = true;
                if !value.is_empty() {
                    current_rules.push((key == "allow", value.to_owned()));
                }
            }
            _ => {},
        }
    }
    finish(&mut grouped_rules, &mut group_agents, &mut current_rules, &mut in_rules);

    let longest_specific = grouped_rules
        .iter()
        .flat_map(|(agents, _)| agents.iter())
        .filter(|agent| agent.as_str() != "*" && requested_agent.starts_with(agent.as_str()))
        .map(|agent| agent.len())
        .max();
    let mut best_rule: Option<(usize, bool)> = None;
    for (agents, rules) in grouped_rules {
        let matching = agents.iter().any(|agent| {
            if let Some(specific_len) = longest_specific {
                agent.as_str() != "*"
                    && agent.len() == specific_len
                    && requested_agent.starts_with(agent)
            } else {
                agent.as_str() == "*"
            }
        });
        if !matching {
            continue;
        }
        for (allow, pattern) in rules {
            if robots_prefix_pattern_matches_v1(&pattern, request_target)? {
                let length = pattern.chars().filter(|c| *c != '*' && *c != '$').count();
                if best_rule.is_none_or(|(existing, existing_allow)| {
                    length > existing || (length == existing && allow && !existing_allow)
                }) {
                    best_rule = Some((length, allow));
                }
            }
        }
    }
    Ok(best_rule.is_none_or(|(_, allow)| allow))
}

fn robots_prefix_pattern_matches_v1(
    pattern: &str,
    target: &str,
) -> Result<bool, RuntimeWebFetchError> {
    if !pattern.starts_with('/') || pattern.len() > 512 || pattern.contains('\\') {
        return Err(RuntimeWebFetchError::RobotsNotAllowed);
    }
    let anchored = pattern.ends_with('$');
    let without_end = if anchored { &pattern[..pattern.len() - 1] } else { pattern };
    if without_end.contains('$') {
        return Err(RuntimeWebFetchError::RobotsNotAllowed);
    }
    let pattern = without_end.as_bytes();
    let target = target.as_bytes();
    // Linear-space, linear-time wildcard matcher. A hostile 64 KiB robots
    // response cannot force a target.len() * pattern.len() DP allocation.
    let mut p = 0;
    let mut t = 0;
    let mut previous_star = None;
    let mut star_consumed_until = 0;
    while t < target.len() {
        if p == pattern.len() && !anchored {
            return Ok(true);
        }
        if p < pattern.len() && pattern[p] == b'*' {
            previous_star = Some(p);
            p += 1;
            star_consumed_until = t;
        } else if p < pattern.len() && pattern[p] == target[t] {
            p += 1;
            t += 1;
        } else if let Some(star) = previous_star {
            star_consumed_until += 1;
            t = star_consumed_until;
            p = star + 1;
        } else {
            return Ok(false);
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    Ok(p == pattern.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_specific_agent_overrides_wildcard_and_obeys_longest_allow() {
        let file = concat!(
            "User-agent: *\nDisallow: /\n\n",
            "User-agent: siteArchitectBot\nDisallow: /private/\nAllow: /private/public/\n",
        );
        assert!(robots_allows_v1(file, BOT_AGENT, "/public/").unwrap());
        assert!(!robots_allows_v1(file, BOT_AGENT, "/private/customer").unwrap());
        assert!(robots_allows_v1(file, BOT_AGENT, "/private/public/index.html").unwrap());
    }

    #[test]
    fn robots_shorter_agent_allow_cannot_override_more_specific_disallow() {
        let file = "User-agent: site\nAllow: /\n\nUser-agent: siteArchitectBot\nDisallow: /\n";
        assert!(!robots_allows_v1(file, BOT_AGENT, "/private").unwrap());
    }

    #[test]
    fn robots_patterns_support_wildcard_and_terminal_anchor() {
        let file = "User-agent: *\nDisallow: /*?token=*\nAllow: /search?token=public$\n";
        assert!(!robots_allows_v1(file, BOT_AGENT, "/search?token=secret").unwrap());
        assert!(robots_allows_v1(file, BOT_AGENT, "/search?token=public").unwrap());
        assert!(robots_allows_v1(file, BOT_AGENT, "/search?other=value").unwrap());
    }

    #[test]
    fn malformed_robots_fails_closed() {
        for text in [
            "User-agent: *\nDisallow: private\n",
            "User-agent: *\nDisallow: /a$b\n",
            "\0",
        ] {
            assert_eq!(
                robots_allows_v1(text, BOT_AGENT, "/a").unwrap_err(),
                RuntimeWebFetchError::RobotsNotAllowed
            );
        }
    }

    #[test]
    fn no_rules_means_allowed() {
        assert!(robots_allows_v1("User-agent: *\nDisallow:\n", BOT_AGENT, "/").unwrap());
    }
}
