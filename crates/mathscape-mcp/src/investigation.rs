//! Investigation identity and staged change-set helpers shared by MCP and
//! the Desktop review commands.

use mathscape_domain::UserChangeOperation;
use mathscape_orchestrator::change_set_plan::StagedChangeSet;
use mathscape_store_sqlite::{PendingChangeSet, ProposedChangeRecord};
use serde_json::json;

/// Investigation id of a sandbox session. One sandbox session is one
/// investigation; dialogue threads are only provenance metadata.
pub fn sandbox_investigation_id(sandbox_session_id: &str) -> String {
    format!("sandbox:{sandbox_session_id}")
}

/// Compiled operations stored on a version at propose time.
pub fn record_operations(record: &ProposedChangeRecord) -> Vec<UserChangeOperation> {
    serde_json::from_value(
        record
            .result
            .get("compiled_operations")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .unwrap_or_default()
}

pub fn staged_change_sets(versions: &[PendingChangeSet]) -> Vec<StagedChangeSet> {
    versions
        .iter()
        .map(|item| StagedChangeSet {
            change_set_id: item.record.change_set_id.clone(),
            version: item.record.version,
            state: item.record.state.clone(),
            created_at: item.record.created_at.clone(),
            operations: record_operations(&item.record),
            depends_on: item
                .dependencies
                .iter()
                .map(|dep| (dep.depends_on_change_set_id.clone(), dep.depends_on_version))
                .collect(),
        })
        .collect()
}
