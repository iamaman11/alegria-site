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
    // The host is case-insensitive, but the path and query may identify
    // entirely different documents. Spain's consular site, for example,
    // serves Schengen and national visas from index.aspx using the scs
    // query parameter. Dropping it silently overwrote nine of ten SERP URLs.
    let trimmed = url.trim();
    let Some(parsed) = parse_url_relaxed(trimmed) else {
        return trimmed.to_ascii_lowercase();
    };

    let Some(host_raw) = parsed.host_str() else {
        return trimmed.to_ascii_lowercase();
    };

    let host = host_to_ascii(strip_www(&host_raw.to_ascii_lowercase()));
    let path = parsed.path().trim_end_matches('/');
    let query = parsed
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    format!("{host}{path}{query}")
}

#[cfg(test)]
mod tests {
    use super::url_norm;

    #[test]
    fn consular_query_parameters_identify_distinct_real_documents() {
        let base = "https://www.exteriores.gob.es/Consulados/moscu/es/ServiciosConsulares/Paginas/index.aspx";
        let schengen = format!(
            "{base}?scca=Visados&scco=Rusia&scd=203&scs=Visado+de+estancia+%28visado+Schengen%29"
        );
        let digital_nomad = format!(
            "{base}?scca=Visados&scco=Rusia&scd=203&scs=Visado+de+residencia+para+teletrabajo"
        );
        assert_ne!(url_norm(&schengen), url_norm(&digital_nomad));
        assert!(url_norm(&schengen).contains("scs=Visado+de+estancia"));
        assert!(url_norm(&digital_nomad).contains("scs=Visado+de+residencia"));
        assert_ne!(url_norm(&schengen), url_norm(base));
    }

    #[test]
    fn host_is_normalized_without_destroying_case_sensitive_source_identity() {
        assert_eq!(
            url_norm("https://WWW.Example.COM/Consulados/Moscu/?Visa=Schengen#part"),
            "example.com/Consulados/Moscu?Visa=Schengen"
        );
        assert_ne!(
            url_norm("https://example.com/?Visa=Schengen"),
            url_norm("https://example.com/?Visa=schengen")
        );
        assert_eq!(
            url_norm("https://www.example.com/path/"),
            url_norm("https://example.com/path")
        );
    }
}
