//! Endpoint discovery helpers shared by the local AI providers
//! (Foundry Local, Ollama, custom OpenAI-compatible endpoints).

/// Extract `scheme://host:port` from the first http(s) URL found in `text`.
pub(crate) fn extract_http_base(text: &str) -> Option<String> {
    let start = text.find("http://").or_else(|| text.find("https://"))?;
    let url = &text[start..];
    let end = url
        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        .unwrap_or(url.len());
    let url = &url[..end];
    let scheme_end = url.find("://")? + 3;
    if url.len() <= scheme_end {
        return None;
    }
    // Cut any path component, keeping scheme://authority only
    let base_end = url[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(url.len());
    Some(url[..base_end].to_string())
}

/// Normalize a user-configured endpoint to a base URL: trim whitespace and
/// trailing slashes, strip a `/v1` suffix (the API root is appended by the
/// clients). Returns None for an effectively empty value.
pub(crate) fn normalize_base_url(endpoint: &str) -> Option<String> {
    let e = endpoint.trim().trim_end_matches('/');
    let e = e.strip_suffix("/v1").unwrap_or(e);
    if e.is_empty() {
        None
    } else {
        Some(e.to_string())
    }
}

/// TCP probe of an endpoint base URL (`scheme://host:port`).
pub(crate) fn probe_endpoint(base: &str) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let Ok(url) = url::Url::parse(base) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let Some(port) = url.port_or_known_default() else {
        return false;
    };
    match (host, port).to_socket_addrs() {
        Ok(mut addrs) => {
            addrs.any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok())
        }
        Err(_) => false,
    }
}

pub(crate) async fn probe_endpoint_async(base: &str) -> bool {
    let base = base.to_string();
    tokio::task::spawn_blocking(move || probe_endpoint(&base))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_base_from_foundry_status_output() {
        let out = "🟢 Model management service is running on http://127.0.0.1:55769/openai/status";
        assert_eq!(
            extract_http_base(out),
            Some("http://127.0.0.1:55769".to_string())
        );
    }

    #[test]
    fn extracts_base_without_path() {
        assert_eq!(
            extract_http_base("endpoint: http://localhost:5273 ready"),
            Some("http://localhost:5273".to_string())
        );
    }

    #[test]
    fn returns_none_when_no_url_present() {
        assert_eq!(extract_http_base("service is not running"), None);
        assert_eq!(extract_http_base("http://"), None);
    }

    #[test]
    fn normalizes_configured_endpoints() {
        assert_eq!(
            normalize_base_url(" http://127.0.0.1:11434/ "),
            Some("http://127.0.0.1:11434".to_string())
        );
        assert_eq!(
            normalize_base_url("https://openrouter.ai/api/v1"),
            Some("https://openrouter.ai/api".to_string())
        );
        assert_eq!(normalize_base_url("   "), None);
        assert_eq!(normalize_base_url("/v1"), None);
    }

    #[test]
    fn probe_parses_host_and_port_from_path_prefixed_urls() {
        let url = url::Url::parse("https://openrouter.ai/api").unwrap();
        assert_eq!(url.host_str(), Some("openrouter.ai"));
        assert_eq!(url.port_or_known_default(), Some(443));

        let url = url::Url::parse("http://127.0.0.1:12345/openai/v1").unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.port_or_known_default(), Some(12345));
    }
}
