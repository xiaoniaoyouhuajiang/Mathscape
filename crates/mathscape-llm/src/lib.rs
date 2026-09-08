//! Provider-neutral LLM transport contracts and adapters.
//!
//! Provider SDK types are intentionally kept behind this crate. EMIR/domain
//! entities and persistent schemas must not depend on a particular LLM vendor.

mod backend;
mod credential;
mod genai_backend;
mod mock;
mod profile;
mod request;
mod response;
mod structured;

pub use backend::*;
pub use credential::*;
pub use genai_backend::*;
pub use mock::*;
pub use profile::*;
pub use request::*;
pub use response::*;
pub use structured::*;
