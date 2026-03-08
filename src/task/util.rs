pub fn trim_protocol_and_slash(url_base: &str) -> String {
    let trimmed = url_base.trim().trim_end_matches('/');
    trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::trim_protocol_and_slash;

    #[test]
    fn trim_protocol_and_slash_removes_http_and_slashes() {
        assert_eq!(
            trim_protocol_and_slash("http://localhost:8085/"),
            "localhost:8085"
        );
    }

    #[test]
    fn trim_protocol_and_slash_removes_https_and_whitespace() {
        assert_eq!(
            trim_protocol_and_slash("  https://pubsub-emulator:8681///  "),
            "pubsub-emulator:8681"
        );
    }

    #[test]
    fn trim_protocol_and_slash_keeps_plain_host() {
        assert_eq!(trim_protocol_and_slash("localhost:8085"), "localhost:8085");
    }
}
