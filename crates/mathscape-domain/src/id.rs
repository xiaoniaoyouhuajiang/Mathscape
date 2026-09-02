use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::DomainError;

/// Governance identity that remains stable across revisions.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StableId(String);

impl StableId {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let Some((scheme, body)) = value.split_once(':') else {
            return Err(DomainError::InvalidStableId {
                value,
                reason: "a URI-like scheme is required",
            });
        };

        let mut scheme_chars = scheme.chars();
        let valid_scheme = scheme_chars
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
            && scheme_chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'));
        if !valid_scheme {
            return Err(DomainError::InvalidStableId {
                value,
                reason: "the scheme is invalid",
            });
        }
        if body.is_empty() {
            return Err(DomainError::InvalidStableId {
                value,
                reason: "the scheme-specific part is empty",
            });
        }
        if value.chars().any(char::is_whitespace) {
            return Err(DomainError::InvalidStableId {
                value,
                reason: "whitespace is not allowed",
            });
        }
        if value.chars().any(char::is_control) {
            return Err(DomainError::InvalidStableId {
                value,
                reason: "control characters are not allowed",
            });
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for StableId {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<String> for StableId {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<StableId> for String {
    fn from(value: StableId) -> Self {
        value.0
    }
}

/// Immutable content revision. The first schema supports SHA-256 revisions.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RevisionId(String);

impl RevisionId {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let Some(digest) = value.strip_prefix("sha256:") else {
            return Err(DomainError::InvalidRevisionId {
                value,
                reason: "expected the sha256:<digest> form",
            });
        };
        if digest.len() != 64 {
            return Err(DomainError::InvalidRevisionId {
                value,
                reason: "a SHA-256 digest must contain 64 hexadecimal digits",
            });
        }
        if !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::InvalidRevisionId {
                value,
                reason: "the digest contains a non-hexadecimal character",
            });
        }

        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RevisionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for RevisionId {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<String> for RevisionId {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RevisionId> for String {
    fn from(value: RevisionId) -> Self {
        value.0
    }
}

/// Content-addressed identity of an atomic accepted patch.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitId(RevisionId);

impl CommitId {
    pub fn from_revision(revision: RevisionId) -> Self {
        Self(revision)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for CommitId {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        RevisionId::parse(value).map(Self)
    }
}

macro_rules! non_empty_text_type {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(DomainError::EmptyValue { field: $field });
                }
                if value.chars().any(char::is_control) {
                    return Err(DomainError::ControlCharacter { field: $field });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = DomainError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

non_empty_text_type!(SchemaVersion, "schema_version");
non_empty_text_type!(ActorRef, "actor");
non_empty_text_type!(Timestamp, "timestamp");

#[cfg(test)]
mod tests {
    use super::*;

    const REVISION: &str =
        "sha256:1886330c162b9cf3461a420f9cc6d42c8a237d265a926aad7c8d30b63048a7c2";

    #[test]
    fn accepts_fixture_stable_ids() {
        for value in [
            "mathmap:theory/real-sequences",
            "emir-fixture:unit/source-b-cesaro-proof",
        ] {
            assert_eq!(StableId::parse(value).unwrap().as_str(), value);
        }
    }

    #[test]
    fn rejects_unscoped_or_whitespace_stable_ids() {
        assert!(StableId::parse("sequence-limit").is_err());
        assert!(StableId::parse("mathmap:sequence limit").is_err());
    }

    #[test]
    fn accepts_and_normalizes_sha256_revision_ids() {
        let upper = REVISION
            .to_ascii_uppercase()
            .replacen("SHA256", "sha256", 1);
        assert_eq!(RevisionId::parse(upper).unwrap().as_str(), REVISION);
    }

    #[test]
    fn rejects_malformed_revision_ids() {
        assert!(RevisionId::parse("sha256:1234").is_err());
        assert!(RevisionId::parse(format!("sha256:{}z", "0".repeat(63))).is_err());
    }
}
