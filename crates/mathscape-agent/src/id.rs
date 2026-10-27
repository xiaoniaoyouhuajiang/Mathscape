use std::fmt;

use serde::{Deserialize, Serialize};

/// Invalid identifier at the agent contract boundary.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid {field} `{value}`: {reason}")]
pub struct IdError {
    pub field: &'static str,
    pub value: String,
    pub reason: &'static str,
}

fn validate(field: &'static str, value: String) -> Result<String, IdError> {
    let reason = if value.is_empty() {
        Some("must not be empty")
    } else if value.chars().any(char::is_whitespace) {
        Some("whitespace is not allowed")
    } else if value.chars().any(char::is_control) {
        Some("control characters are not allowed")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(IdError {
            field,
            value,
            reason,
        }),
        None => Ok(value),
    }
}

macro_rules! opaque_id {
    ($(#[$meta:meta])* $name:ident, $field:literal) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
                validate($field, value.into()).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id!(
    /// Round identity, injected by the adapter; agents can never supply it.
    RoundId,
    "round_id"
);
opaque_id!(
    /// Turn identity within a round (a round may contain several turns).
    TurnId,
    "turn_id"
);
opaque_id!(
    /// Pairs a `tool_call` with its `tool_result`.
    CallId,
    "call_id"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_whitespace_and_control() {
        assert!(RoundId::parse("").is_err());
        assert!(TurnId::parse("a b").is_err());
        assert!(CallId::parse("a\u{7}").is_err());
        assert_eq!(RoundId::parse("round-1").unwrap().as_str(), "round-1");
    }

    #[test]
    fn deserialization_validates() {
        assert!(serde_json::from_str::<RoundId>("\"\"").is_err());
        let id: CallId = serde_json::from_str("\"c1\"").unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"c1\"");
    }
}
