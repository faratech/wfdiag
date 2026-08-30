//! Endpoint discovery helpers shared by the local AI providers
//! (Foundry Local, Ollama, custom OpenAI-compatible endpoints).

/// Extract `scheme://host:port` from the first http(s) URL found in `text`.
pub(crate) fn extract_http_base(text: &str) -> Option<String> {
    let start = text.find("http://").or_else(|| text.find("https://"))?;
    let url = &text[start..];
    let end = url
        .find(|character: char| character.is_whitespace() || character == '"' || character == '\'')
        .unwrap_or(url.len());
    let url = &url[..end];
    let scheme_end = url.find("://")? + 3;
    if url.len() <= scheme_end {
        return None;
    }
    let base_end = url[scheme_end..]
        .find('/')
        .map(|index| scheme_end + index)
        .unwrap_or(url.len());
    Some(url[..base_end].to_string())
}

pub(crate) fn normalize_base_url(endpoint: &str) -> Option<String> {
    wfdiag_native_ai_provider::normalize_base_url(endpoint)
}

pub(crate) async fn probe_endpoint_async(base: &str) -> bool {
    wfdiag_native_ai_provider::probe_http_endpoint_async(base).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_base_from_foundry_status_output() {
        let output =
            "🟢 Model management service is running on http://127.0.0.1:55769/openai/status";
        assert_eq!(
            extract_http_base(output),
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
