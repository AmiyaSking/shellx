use regex::Regex;

/// Masks credentials that might appear in command output before it is handed
/// back to an agent (or shown in the audit log).
pub struct Redactor {
    rules: Vec<(Regex, &'static str)>,
}

impl Redactor {
    pub fn new() -> Self {
        let rules = [
            (
                r"(?i)(password|passwd|secret|token|api[_-]?key)\s*[=:]\s*[^\s&,;]+",
                "$1=[REDACTED]",
            ),
            (
                r"(?is)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
                "[REDACTED PRIVATE KEY]",
            ),
            (r"\bAKIA[0-9A-Z]{16}\b", "[REDACTED AWS KEY]"),
            (r"\bgh[pousr]_[A-Za-z0-9]{20,}\b", "[REDACTED TOKEN]"),
            (r"(?i)\bBearer\s+[A-Za-z0-9._\-]+", "Bearer [REDACTED]"),
        ]
        .into_iter()
        .map(|(pattern, replacement)| {
            (
                Regex::new(pattern).expect("built-in redaction regex must compile"),
                replacement,
            )
        })
        .collect();

        Self { rules }
    }

    pub fn redact(&self, input: &str) -> String {
        let mut output = input.to_string();
        for (rule, replacement) in &self.rules {
            output = rule.replace_all(&output, *replacement).to_string();
        }
        output
    }
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Redactor;

    #[test]
    fn masks_key_value_secrets() {
        let redactor = Redactor::new();
        let output = redactor.redact("password=hunter2 token: abc123");
        assert!(!output.contains("hunter2"));
        assert!(!output.contains("abc123"));
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn masks_private_key_blocks() {
        let redactor = Redactor::new();
        let output = redactor.redact(
            "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nabc123\n-----END OPENSSH PRIVATE KEY-----\nafter",
        );
        assert!(!output.contains("abc123"));
        assert!(output.contains("[REDACTED PRIVATE KEY]"));
    }

    #[test]
    fn masks_bearer_tokens() {
        let redactor = Redactor::new();
        let output = redactor.redact("Authorization: Bearer eyJhbGciOi.payload.sig");
        assert!(!output.contains("eyJhbGciOi"));
        assert!(output.contains("Bearer [REDACTED]"));
    }
}
