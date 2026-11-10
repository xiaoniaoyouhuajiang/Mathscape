//! Investigation identity and the single `scope=investigation` projection.
//!
//! `graph.get`, `graph.neighborhood` (MCP) and the Desktop map all call
//! [`investigation_landscape`]: the accepted landscape with the
//! investigation's active `staged` versions overlaid. Nothing here writes;
//! staged objects never enter the accepted head, exports, default search or
//! the formal edit entry points.

use std::collections::BTreeMap;

use mathscape_domain::{
    ActorRef, EntityKind, LifecycleStatus, MapEditObject, RevisionId, StableId, UserChangeOperation,
};
use mathscape_orchestrator::change_set_plan::{
    InvestigationOverlay, StagedChangeSet, project_investigation,
};
use mathscape_store_sqlite::{
    LandscapeSnapshot, PendingChangeSet, ProposedChangeRecord, SourceMapSnapshot,
    SqliteArtifactStore, SqliteStoreError, StoredMapEndpoint, StoredMapObject,
};
use serde::Serialize;
use serde_json::{Value, json};

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
            operations: record_operations(&item.record),
            depends_on: item
                .dependencies
                .iter()
                .map(|dep| (dep.depends_on_change_set_id.clone(), dep.depends_on_version))
                .collect(),
        })
        .collect()
}

/// Projection metadata of one node or relation (`staged: true`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StagedMark {
    pub staged: bool,
    pub change_set_id: String,
    pub change_set_version: i64,
    /// Every staged version that creates or edits the object.
    pub change_sets: Vec<String>,
    /// Accepted revision a staged edit replaces (`None` for creations).
    pub replaces_revision: Option<String>,
}

/// An accepted object with mutually exclusive staged replacements; the
/// projection shows the accepted version.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StagedConflictMark {
    pub object_id: String,
    pub change_sets: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct InvestigationMarks {
    pub staged: BTreeMap<String, StagedMark>,
    pub conflicts: Vec<StagedConflictMark>,
}

fn key_label((id, version): &(String, i64)) -> String {
    format!("{id}@{version}")
}

pub fn investigation_overlay(
    store: &SqliteArtifactStore,
    investigation_id: &str,
) -> Result<InvestigationOverlay, SqliteStoreError> {
    let versions = store.list_change_set_versions(investigation_id)?;
    let sets = staged_change_sets(&versions);
    let snapshot = store.map_edit_snapshot()?;
    let actor = ActorRef::parse("mathscape-investigation-projection")?;
    Ok(project_investigation(&snapshot, &sets, &actor))
}

fn stored_object(object: &MapEditObject) -> Option<(StoredMapObject, Vec<StoredMapEndpoint>)> {
    let kind: EntityKind = serde_json::from_value(Value::String(object.kind.clone())).ok()?;
    let lifecycle: LifecycleStatus =
        serde_json::from_value(Value::String(object.lifecycle.clone())).ok()?;
    let endpoints = object
        .endpoints
        .iter()
        .map(|endpoint| StoredMapEndpoint {
            role: endpoint.role().as_str().to_owned(),
            ordinal: endpoint.ordinal(),
            entity_id: endpoint.entity().clone(),
        })
        .collect();
    Some((
        StoredMapObject {
            stable_id: StableId::parse(object.object_id.clone()).ok()?,
            revision_id: RevisionId::parse(&object.revision).ok()?,
            kind,
            relation: !object.endpoints.is_empty(),
            payload: object.payload.clone(),
            lifecycle,
        },
        endpoints,
    ))
}

/// Overlays staged objects onto an accepted source map (replacing accepted
/// objects that a staged edit revises) and returns the marks.
pub fn overlay_source_map(
    source_map: &mut SourceMapSnapshot,
    overlay: &InvestigationOverlay,
) -> InvestigationMarks {
    let mut marks = InvestigationMarks::default();
    for staged in &overlay.objects {
        let Some((object, endpoints)) = stored_object(&staged.object) else {
            continue;
        };
        if let Some(position) = source_map
            .objects
            .iter()
            .position(|candidate| candidate.stable_id == object.stable_id)
        {
            source_map.objects[position] = object.clone();
        } else {
            source_map.objects.push(object.clone());
        }
        if object.relation {
            source_map
                .relation_endpoints
                .insert(object.stable_id.clone(), endpoints);
        }
        marks.staged.insert(
            staged.object.object_id.clone(),
            StagedMark {
                staged: true,
                change_set_id: staged.change_set_id.clone(),
                change_set_version: staged.version,
                change_sets: staged.change_sets.iter().map(key_label).collect(),
                replaces_revision: staged.replaces_revision.clone(),
            },
        );
    }
    marks.conflicts = overlay
        .conflicts
        .iter()
        .map(|conflict| StagedConflictMark {
            object_id: conflict.object_id.clone(),
            change_sets: conflict.change_sets.iter().map(key_label).collect(),
        })
        .collect();
    marks
}

/// The accepted landscape with the investigation overlay applied. This is
/// the one projection shared by MCP `graph.*` and the Desktop map.
pub fn investigation_landscape(
    store: &SqliteArtifactStore,
    investigation_id: &str,
) -> Result<(LandscapeSnapshot, InvestigationMarks), SqliteStoreError> {
    let landscape = store.landscape_snapshot()?;
    let overlay = investigation_overlay(store, investigation_id)?;
    let mut sources = landscape.sources().clone();
    let marks = overlay_source_map(&mut sources, &overlay);
    Ok((landscape.with_sources(sources), marks))
}
