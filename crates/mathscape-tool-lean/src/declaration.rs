use serde::{Deserialize, Serialize};

pub const DECLARATION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationKind {
    Unknown,
    Axiom,
    Definition,
    Theorem,
    Opaque,
    Quotient,
    Inductive,
    Constructor,
    Recursor,
}

/// A declaration candidate exported from one concrete Lean environment.
///
/// This is a retrieval record, not a proof certificate. The `profile_fingerprint`
/// binds it to the environment in which the declaration was observed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarationCandidate {
    pub schema_version: u32,
    pub profile_fingerprint: String,
    pub name: String,
    pub kind: DeclarationKind,
    pub module: String,
    #[serde(rename = "type")]
    pub type_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_column: Option<u32>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub type_truncated: bool,
    #[serde(default)]
    pub metadata_complete: bool,
}

impl DeclarationCandidate {
    pub fn is_theorem_like(&self) -> bool {
        matches!(
            self.kind,
            DeclarationKind::Theorem | DeclarationKind::Axiom | DeclarationKind::Opaque
        )
    }
}
