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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_secret() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("123"), "***");
        assert_eq!(mask_secret("12345678"), "123***678");
    }
}
