//! Reproducible Marker requests and controlled external-process execution.
//!
//! This crate consumes validated PDF selections. It never interprets mathematics
//! and has no authority to register artifacts or mutate the Mathscape map.

mod error;
mod execution;
mod installation;
mod inventory;
mod page_range;
mod probe;
mod profile;
mod request;
mod staging;

pub use error::{MarkerExecutionError, MarkerProductScanError, MarkerRequestError};
pub use execution::{
    CapturedLog, MarkerCancellation, MarkerExecutionOptions, MarkerRun, MarkerRunStatus,
    MarkerRunner,
};
pub use installation::MarkerInstallation;
pub use inventory::{
    MARKER_PRODUCT_INVENTORY_SCHEMA_VERSION, MarkerProduct, MarkerProductDigest,
    MarkerProductInventory, MarkerProductIssue, MarkerProductRole, MarkerProductScanLimits,
    MarkerProductScanner,
};
pub use page_range::MarkerPageRange;
pub use probe::{MarkerProbe, MarkerVersion};
pub use profile::{InlineMathPolicy, MarkerMode, MarkerProfile};
pub use request::{MarkerCommandSpec, MarkerRequest};
pub use staging::{MarkerStagingArea, MarkerStagingDirectory};
