//! Public MCP envelope DTOs (contract §2, `mcp_envelope v0`).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MCP_ENVELOPE_VERSION: &str = "mcp_envelope v0";

/// Business result status carried at the top level of every tool's structured
/// result (§2.2). These are normal MCP results, not JSON-RPC errors.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Ok,
    NotFound,
    Ambiguous,
    /// Valid result cut at a hard cap; the tool also sets `truncated: true`.
    Truncated,
    /// A required external capability is missing or has the wrong version;
    /// `details` carries the [`crate::CapabilityProbe`].
    CapabilityUnavailable,
    /// A write-like proposal passed validation and is waiting for a user
    /// decision. Phase2 only.
    Staged,
    /// A write-like proposal was retained with validation debt and cannot be
    /// accepted. Phase2 only.
    Blocked,
}

impl ToolStatus {
    pub const ALL: [Self; 7] = [
        Self::Ok,
        Self::NotFound,
        Self::Ambiguous,
        Self::Truncated,
        Self::CapabilityUnavailable,
        Self::Staged,
        Self::Blocked,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::NotFound => "not_found",
            Self::Ambiguous => "ambiguous",
            Self::Truncated => "truncated",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::Staged => "staged",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for ToolStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Call-error codes (§2.2). `not_found`, `staged` and `blocked` are business
/// [`ToolStatus`] values; `blocked` is not a call error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode {
    InvalidArg,
    Forbidden,
    Internal,
}

impl ToolErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArg => "invalid_arg",
            Self::Forbidden => "forbidden",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ToolErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Call error body: `{ code, message, details }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("{code}: {message}")]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ToolError {
    pub fn new(code: ToolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn into_response(self) -> ToolErrorResponse {
        ToolErrorResponse { error: self }
    }
}

/// Wire wrapper: `{ "error": { code, message, details } }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolErrorResponse {
    pub error: ToolError,
}

/// Material locator (§2.3), externally tagged:
/// `{"md_byte_range":{"start":0,"end":10}}` or `{"pdf_pages":{"from":1,"to":2}}`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Locator {
    /// Byte offsets, half-open `[start, end)`.
    MdByteRange { start: u64, end: u64 },
    /// 1-based page numbers, closed `[from, to]`. (PDF-internal
    /// `pdf_page_index` values elsewhere are 0-based.)
    PdfPages { from: u32, to: u32 },
}

impl Locator {
    /// Structural check only; bounds against the actual material are the
    /// tool's job.
    pub fn validate(&self) -> Result<(), ToolError> {
        match *self {
            Self::MdByteRange { start, end } if start > end => Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                format!("md_byte_range start {start} is after end {end}"),
            )),
            Self::PdfPages { from: 0, .. } => Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "pdf_pages are 1-based; `from` must be >= 1",
            )),
            Self::PdfPages { from, to } if from > to => Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                format!("pdf_pages from {from} is after to {to}"),
            )),
            _ => Ok(()),
        }
    }
}

/// Graph scope (§2.3). Phase1 implements only `accepted`; `investigation`
/// is answered as `accepted` with a `scope_not_supported` diagnostic.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Accepted,
    Investigation,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tool_status_wire_names() {
        let names: Vec<_> = ToolStatus::ALL
            .iter()
            .map(|s| serde_json::to_value(s).unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                json!("ok"),
                json!("not_found"),
                json!("ambiguous"),
                json!("truncated"),
                json!("capability_unavailable"),
                json!("staged"),
                json!("blocked")
            ]
        );
        for status in ToolStatus::ALL {
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                json!(status.as_str())
            );
        }
    }

    #[test]
    fn error_envelope_shape() {
        let response = ToolError::new(ToolErrorCode::InvalidArg, "bad locator")
            .with_details(json!({"field": "locator"}))
            .into_response();
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(
            value,
            json!({"error": {"code": "invalid_arg", "message": "bad locator", "details": {"field": "locator"}}})
        );
        let back: ToolErrorResponse = serde_json::from_value(value).unwrap();
        assert_eq!(back, response);
        assert!(serde_json::from_value::<ToolErrorCode>(json!("blocked")).is_err());
        assert!(serde_json::from_value::<ToolErrorCode>(json!("not_found")).is_err());
    }

    #[test]
    fn locator_shapes() {
        let md = Locator::MdByteRange {
            start: 0,
            end: 1200,
        };
        assert_eq!(
            serde_json::to_value(md).unwrap(),
            json!({"md_byte_range": {"start": 0, "end": 1200}})
        );
        let pdf: Locator =
            serde_json::from_value(json!({"pdf_pages": {"from": 1, "to": 3}})).unwrap();
        assert_eq!(pdf, Locator::PdfPages { from: 1, to: 3 });
        assert!(
            serde_json::from_value::<Locator>(
                json!({"md_byte_range": {"start": 0, "end": 1}, "pdf_pages": {"from": 1, "to": 1}})
            )
            .is_err()
        );
        assert!(md.validate().is_ok());
        assert!(
            Locator::MdByteRange { start: 5, end: 4 }
                .validate()
                .is_err()
        );
        assert!(Locator::PdfPages { from: 0, to: 1 }.validate().is_err());
        assert!(Locator::PdfPages { from: 3, to: 2 }.validate().is_err());
    }

    #[test]
    fn scope_defaults_to_accepted() {
        assert_eq!(Scope::default(), Scope::Accepted);
        assert_eq!(
            serde_json::to_value(Scope::Investigation).unwrap(),
            json!("investigation")
        );
    }
}
