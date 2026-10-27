use std::fmt;

use serde::{Deserialize, Serialize};

/// Harness behind an [`crate::AgentExecutor`] (contract §5).
///
/// Serialized as snake_case (`codex`, `claude_code`, `hmz`, `custom`).
/// `Hmz` is an optional adapter and never required by a default flow.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentBackend {
    Codex,
    ClaudeCode,
    Hmz,
    Custom,
}

impl AgentBackend {
    pub const ALL: [Self; 4] = [Self::Codex, Self::ClaudeCode, Self::Hmz, Self::Custom];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::Hmz => "hmz",
            Self::Custom => "custom",
        }
    }

    /// Capability that must be available for this backend, if any.
    pub const fn required_capability(self) -> Option<crate::Capability> {
        match self {
            Self::Codex => Some(crate::Capability::Codex),
            Self::ClaudeCode => Some(crate::Capability::ClaudeCode),
            Self::Hmz => Some(crate::Capability::Hmz),
            Self::Custom => None,
        }
    }
}

impl fmt::Display for AgentBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_match_as_str() {
        for backend in AgentBackend::ALL {
            let json = serde_json::to_string(&backend).unwrap();
            assert_eq!(json, format!("\"{}\"", backend.as_str()));
            assert_eq!(
                serde_json::from_str::<AgentBackend>(&json).unwrap(),
                backend
            );
        }
    }
}
