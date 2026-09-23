//! Hostname allowlist patterns: an exact name or a `*` glob.
//!
//! Matching is against the whole hostname, ASCII case-insensitive, and `*`
//! matches any run of characters including dots (so `*.corp.example`
//! matches `a.b.corp.example` but not `corp.example`). Callers strip a
//! trailing dot from fully-qualified inputs themselves.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostnamePattern {
    /// Lowercased pattern split on `*`; a single element means no wildcard.
    literals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    Empty,
    InvalidChar(char),
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("pattern must not be empty"),
            Self::InvalidChar(c) => write!(f, "invalid character {c:?} in hostname pattern"),
        }
    }
}

impl std::error::Error for PatternError {}

impl HostnamePattern {
    pub fn compile(pattern: &str) -> Result<Self, PatternError> {
        if pattern.is_empty() {
            return Err(PatternError::Empty);
        }
        if let Some(c) = pattern
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '*')))
        {
            return Err(PatternError::InvalidChar(c));
        }
        Ok(Self {
            literals: pattern
                .to_ascii_lowercase()
                .split('*')
                .map(str::to_owned)
                .collect(),
        })
    }

    pub fn is_match(&self, hostname: &str) -> bool {
        let hostname = hostname.to_ascii_lowercase();
        let (first, rest) = self.literals.split_first().expect("at least one literal");
        let Some((last, middle)) = rest.split_last() else {
            return hostname == *first;
        };
        let Some(mut cursor) = hostname
            .strip_prefix(first.as_str())
            .and_then(|h| h.strip_suffix(last.as_str()))
        else {
            return false;
        };
        for literal in middle {
            match cursor.find(literal.as_str()) {
                Some(idx) => cursor = &cursor[idx + literal.len()..],
                None => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, hostname: &str) -> bool {
        HostnamePattern::compile(pattern)
            .unwrap()
            .is_match(hostname)
    }

    #[test]
    fn exact_match() {
        assert!(matches("api.example.com", "api.example.com"));
        assert!(matches("api.example.com", "API.EXAMPLE.COM"));
        assert!(!matches("api.example.com", "api.example.com.evil.com"));
        assert!(!matches("api.example.com", "evil-api.example.com"));
        assert!(!matches("api.example.com", "example.com"));
        assert!(!matches("api.example.com", "api.example.com."));
    }

    #[test]
    fn wildcard_match() {
        assert!(matches("*.example.com", "api.example.com"));
        assert!(matches("*.example.com", "sub.api.example.com"));
        assert!(!matches("*.example.com", "example.com"));
        assert!(!matches("*.example.com", "example.com.evil.com"));
        assert!(matches("*", "anything.at.all"));
        assert!(matches("git*.corp", "git.corp"));
        assert!(matches("git*.corp", "gitlab.corp"));
        assert!(!matches("git*.corp", "lab.corp"));
        assert!(matches("*.a.*.b", "x.a.y.b"));
        assert!(matches("*.a.*.b", ".a..b"));
        assert!(!matches("*.a.*.b", "x.a.b"));
        assert!(matches("a*a", "aa"));
        assert!(!matches("a*a", "a"));
        assert!(matches("a**b", "ab"));
        assert!(matches("a**b", "axyb"));
    }

    #[test]
    fn non_ascii_hostnames_never_panic() {
        for pattern in ["a*b", "*", "a*", "*b", "*.a.*.b", "aé"] {
            let Ok(pattern) = HostnamePattern::compile(pattern) else {
                continue;
            };
            for hostname in ["aé", "aéb", "é", "a\u{300}b", "\u{10FFFF}"] {
                let _ = pattern.is_match(hostname);
            }
        }
        assert!(!matches("a*b", "aé"));
        assert!(!matches("a*b", "éb"));
        assert!(matches("a*b", "aéb"));
    }

    #[test]
    fn rejects_bad_patterns() {
        assert_eq!(HostnamePattern::compile(""), Err(PatternError::Empty));
        assert_eq!(
            HostnamePattern::compile("a.example.com/"),
            Err(PatternError::InvalidChar('/'))
        );
        assert!(HostnamePattern::compile("host:443").is_err());
        assert!(HostnamePattern::compile("a b").is_err());
    }
}
