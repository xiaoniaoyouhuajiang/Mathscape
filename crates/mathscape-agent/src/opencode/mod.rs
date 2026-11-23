//! OpenCode via ACP. Native file tools are on; shell is off by default and
//! per-command approval is not implemented yet. Network tools are denied.
//!
//! Isolation is weaker than Codex's: this is approval-layer path validation,
//! not OS enforcement. Checks cannot prevent path replacement between approval
//! and use, or tools executing without asking. No bwrap is used.

mod approval;
mod config;
mod fixture;
mod host;
mod session;

pub use config::OpenCodeConfig;
pub use fixture::serve_acp_fixture;
pub use session::OpenCodeExecutor;
