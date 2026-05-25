pub fn websocket_url(server_url: &str, token: &str) -> anyhow::Result<String> {
    let mut base = server_url.trim_end_matches('/').to_string();
    if let Some(rest) = base.strip_prefix("https://") {
        base = format!("wss://{rest}");
    } else if let Some(rest) = base.strip_prefix("http://") {
        base = format!("ws://{rest}");
    }
    Ok(format!(
        "{base}/v1/bridge/ws?token={}",
        encode_query_component(token)
    ))
}

pub fn redacted_url(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?token=<redacted>"),
        None => url.to_string(),
    }
}

fn encode_query_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_converts_http_to_ws() {
        let url = websocket_url("http://127.0.0.1:8787/", "change-me").unwrap();

        assert_eq!(url, "ws://127.0.0.1:8787/v1/bridge/ws?token=change-me");
    }

    #[test]
    fn websocket_url_encodes_token_query_value() {
        let url = websocket_url("https://agent.example", "a token&with=query").unwrap();

        assert_eq!(
            url,
            "wss://agent.example/v1/bridge/ws?token=a%20token%26with%3Dquery"
        );
    }

    #[test]
    fn redacted_url_removes_query_token() {
        let url = redacted_url("wss://agent.example/v1/bridge/ws?token=secret");

        assert_eq!(url, "wss://agent.example/v1/bridge/ws?token=<redacted>");
    }

    #[test]
    fn encode_query_component_escapes_reserved_and_unicode() {
        assert_eq!(encode_query_component("a+b/c?d"), "a%2Bb%2Fc%3Fd");
        assert_eq!(encode_query_component("café"), "caf%C3%A9");
        assert_eq!(encode_query_component("AZaz09-._~"), "AZaz09-._~");
        assert_eq!(encode_query_component("100%"), "100%25");
    }
}
