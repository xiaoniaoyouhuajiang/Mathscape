use crate::{ContentHash, StableId};
use serde::{Deserialize, Serialize};

/// User disposition only; neither action accepts source entities or commits a map.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryDispositionRequest {
    pub run_id: StableId,
    pub request_id: StableId,
    pub expected_result_fingerprint: ContentHash,
    pub expected_disposition_request_id: Option<StableId>,
    pub reason: String,
}
