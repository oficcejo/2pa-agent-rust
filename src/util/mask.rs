/// Mask API key or secret token for safe logging/display.
pub fn mask_secret(secret: &str) -> String {
    let s = secret.trim();
    if s.is_empty() {
        return "".to_string();
    }
    if s.len() <= 6 {
        return "***".to_string();
    }
    format!("{}***{}", &s[..3], &s[s.len() - 3..])
}

/// Replace known secrets in free-form error text.
///
/// Also replaces common URL-encoded forms so a leaked `Authorization` or
/// query-string secret cannot reappear through `reqwest`/URL formatting.
pub fn redact_secrets(message: &str, secrets: &[&str]) -> String {
    let mut out = message.to_string();
    for secret in secrets {
        let s = secret.trim();
        if s.is_empty() {
            continue;
        }
        let encoded = urlencoding::encode(s);
        for variant in [s.to_string(), encoded.into_owned()] {
            if variant.is_empty() {
                continue;
            }
            out = out.replace(&variant, "[redacted]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_secret() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("123"), "***");
        assert_eq!(mask_secret("12345678"), "123***678");
    }

    #[test]
    fn test_redact_secrets_plain_and_encoded() {
        let msg = "Authorization: Bearer abc123secret and q=abc123secret and e=abc123secret%21";
        let out = redact_secrets(msg, &["abc123secret!", "abc123secret"]);
        assert!(!out.contains("abc123secret"));
        assert!(out.contains("[redacted]"));
    }
}
