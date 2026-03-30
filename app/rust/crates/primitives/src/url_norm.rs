use url::Url;

fn strip_www(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

fn host_to_ascii(host: &str) -> String {
    idna::domain_to_ascii(host).unwrap_or_else(|_| host.to_string())
}

fn parse_url_relaxed(input: &str) -> Option<Url> {
    if let Ok(u) = Url::parse(input) {
        return Some(u);
    }
    let with_scheme = format!("https://{input}");
    Url::parse(&with_scheme).ok()
}

pub fn domain_norm(url_or_host: &str) -> String {
    if url_or_host.trim().is_empty() {
        return String::new();
    }
    let lowered = url_or_host.trim().to_lowercase();

    let raw_host = parse_url_relaxed(&lowered)
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| lowered.split('/').next().unwrap_or("").to_string());

    let host = strip_www(&raw_host);
    host_to_ascii(host)
}

pub fn url_norm(url: &str) -> String {
    if url.trim().is_empty() {
        return String::new();
    }
    let lowered = url.trim().to_lowercase();

    let Some(parsed) = parse_url_relaxed(&lowered) else {
        return lowered;
    };

    let Some(host_raw) = parsed.host_str() else {
        return lowered;
    };

    let host = host_to_ascii(strip_www(host_raw));
    let mut path = parsed.path().trim_end_matches('/').to_string();
    if path == "/" {
        path.clear();
    }
    format!("{host}{path}")
}
