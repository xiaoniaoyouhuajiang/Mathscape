//! Reproducible Marker requests and controlled external-process execution.
//!
//! This crate consumes validated PDF selections. It never interprets mathematics
//! and has no authority to register artifacts or mutate the Mathscape map.

mod error;
mod execution;
mod installation;
mod page_range;
mod probe;
mod profile;
mod request;
mod staging;

pub use error::{MarkerExecutionError, MarkerRequestError};
pub use execution::{
    CapturedLog, MarkerCancellation, MarkerExecutionOptions, MarkerRun, MarkerRunStatus,
    MarkerRunner,
};
pub use installation::MarkerInstallation;
pub use page_range::MarkerPageRange;
pub use probe::{MarkerProbe, MarkerVersion};
pub use profile::{InlineMathPolicy, MarkerMode, MarkerProfile};
pub use request::{MarkerCommandSpec, MarkerRequest};
pub use staging::{MarkerStagingArea, MarkerStagingDirectory};
