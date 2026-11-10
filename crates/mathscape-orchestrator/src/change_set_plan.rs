//! Pure planning helpers for Phase2 staged change sets.
//!
//! Storage, MCP and Desktop all hand plain records to these functions so the
//! dependency closure, the topological order, SourceSpan de-duplication and
//! the investigation projection are computed by one implementation.

use std::collections::{BTreeMap, BTreeSet};

use mathscape_domain::{
    ActorRef, MapEditObject, MapEditSnapshot, UserChangeOperation, UserChangeRequest,
};

use crate::map_edit::plan_map_change_over_head;

/// Key of one immutable change set version.
pub type ChangeSetKey = (String, i64);

/// One change set version as the planner sees it.
#[derive(Clone, Debug)]
pub struct StagedChangeSet {
    pub change_set_id: String,
    pub version: i64,
    /// `staged` | `blocked` | `accepted` | `discarded` | `superseded` | `withdrawn`.
    pub state: String,
    pub operations: Vec<UserChangeOperation>,
    /// Direct dependencies `(change_set_id, version)` recorded at propose time.
    pub depends_on: Vec<ChangeSetKey>,
}

impl StagedChangeSet {
    pub fn key(&self) -> ChangeSetKey {
        (self.change_set_id.clone(), self.version)
    }
}

/// Why a dependency closure cannot be planned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClosureError {
    /// A dependency version is unknown to the caller.
    Missing(ChangeSetKey),
    /// A dependency is neither accepted nor plannable (blocked, discarded, superseded...).
    NotStaged {
        key: ChangeSetKey,
        state: String,
    },
    /// A dependency is staged but must be accepted first or in the same batch.
    NotAccepted(ChangeSetKey),
    Cycle(Vec<ChangeSetKey>),
}

impl ClosureError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Missing(_) => "dangling_staged_ref",
            Self::NotStaged { .. } => "dependency_blocked",
            Self::NotAccepted(_) => "dependency_not_accepted",
            Self::Cycle(_) => "change_set_dependency_cycle",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::Missing((id, v)) => format!("依赖的 change_set {id}@{v} 不存在"),
            Self::NotStaged {
                key: (id, v),
                state,
            } => {
                format!("依赖的 change_set {id}@{v} 当前为 {state}，不是 staged")
            }
            Self::NotAccepted((id, v)) => {
                format!("依赖的 change_set {id}@{v} 尚未接受，也不在同一批次中")
            }
            Self::Cycle(keys) => format!(
                "change_set 依赖成环：{}",
                keys.iter()
                    .map(|(id, v)| format!("{id}@{v}"))
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
        }
    }
}

/// Topologically orders `members` (dependencies first). Edges to keys outside
/// `members` are ignored. Ties are broken by `(change_set_id, version)` only:
/// the order — and therefore the combined request and its plan fingerprint —
/// is a function of the dependency graph, never of `created_at`, the
/// selection order or the order rows were read in.
pub fn topological_order<'a>(
    members: &[&'a StagedChangeSet],
) -> Result<Vec<&'a StagedChangeSet>, ClosureError> {
    let keys: BTreeSet<ChangeSetKey> = members.iter().map(|set| set.key()).collect();
    let mut remaining: Vec<&StagedChangeSet> = members.to_vec();
    remaining.sort_by_key(|set| set.key());
    let mut done = BTreeSet::<ChangeSetKey>::new();
    let mut ordered = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let Some(position) = remaining.iter().position(|set| {
            set.depends_on
                .iter()
                .filter(|dep| keys.contains(*dep))
                .all(|dep| done.contains(dep))
        }) else {
            return Err(ClosureError::Cycle(
                remaining.iter().map(|set| set.key()).collect(),
            ));
        };
        let set = remaining.remove(position);
        done.insert(set.key());
        ordered.push(set);
    }
    Ok(ordered)
}

/// The not-yet-accepted transitive dependencies of `roots` (roots excluded),
/// in topological order. Accepted dependencies are already part of the head.
/// Every other dependency must be `staged`.
pub fn staged_dependency_closure<'a>(
    all: &'a [StagedChangeSet],
    roots: &[ChangeSetKey],
) -> Result<Vec<&'a StagedChangeSet>, ClosureError> {
    let index: BTreeMap<ChangeSetKey, &StagedChangeSet> =
        all.iter().map(|set| (set.key(), set)).collect();
    let root_keys: BTreeSet<ChangeSetKey> = roots.iter().cloned().collect();
    let mut seen = BTreeSet::<ChangeSetKey>::new();
    let mut stack: Vec<ChangeSetKey> = roots
        .iter()
        .filter_map(|key| index.get(key))
        .flat_map(|set| set.depends_on.iter().cloned())
        .collect();
    let mut members = Vec::new();
    while let Some(key) = stack.pop() {
        if root_keys.contains(&key) {
            return Err(ClosureError::Cycle(vec![key]));
        }
        if !seen.insert(key.clone()) {
            continue;
        }
        let set = index.get(&key).ok_or(ClosureError::Missing(key.clone()))?;
        match set.state.as_str() {
            "accepted" => continue,
            "staged" => {}
            state => {
                return Err(ClosureError::NotStaged {
                    key,
                    state: state.to_owned(),
                });
            }
        }
        members.push(*set);
        stack.extend(set.depends_on.iter().cloned());
    }
    topological_order(&members)
}

/// Checks that a selection may be accepted together: every direct dependency
/// of a selected version is accepted already or selected as well (which makes
/// the selection closed under dependencies). Returns the selection in
/// topological order.
pub fn ordered_selection<'a>(
    all: &'a [StagedChangeSet],
    selection: &[ChangeSetKey],
) -> Result<Vec<&'a StagedChangeSet>, ClosureError> {
    let index: BTreeMap<ChangeSetKey, &StagedChangeSet> =
        all.iter().map(|set| (set.key(), set)).collect();
    let selected: BTreeSet<ChangeSetKey> = selection.iter().cloned().collect();
    let mut members = Vec::new();
    for key in &selected {
        let set = index.get(key).ok_or(ClosureError::Missing(key.clone()))?;
        for dependency in &set.depends_on {
            if selected.contains(dependency) {
                continue;
            }
            match index.get(dependency) {
                Some(dep) if dep.state == "accepted" => {}
                Some(dep) if dep.state == "staged" => {
                    return Err(ClosureError::NotAccepted(dependency.clone()));
                }
                Some(dep) => {
                    return Err(ClosureError::NotStaged {
                        key: dependency.clone(),
                        state: dep.state.clone(),
                    });
                }
                None => return Err(ClosureError::Missing(dependency.clone())),
            }
        }
        members.push(*set);
    }
    topological_order(&members)
}

/// Concatenates the operations of `sets` in the given order. SourceSpan
/// identities are content-addressed, so a later `CreateSourceSpan` for an id
/// already created earlier in the same request is the same span and is
/// dropped; every other repeated target is left for the planner to reject.
pub fn combined_operations<'a>(
    sets: impl IntoIterator<Item = &'a StagedChangeSet>,
    extra: &[UserChangeOperation],
) -> Vec<UserChangeOperation> {
    let mut spans = BTreeSet::<String>::new();
    let mut operations = Vec::new();
    for operation in sets
        .into_iter()
        .flat_map(|set| set.operations.iter())
        .chain(extra.iter())
    {
        if let UserChangeOperation::CreateSourceSpan { object_id, .. } = operation
            && !spans.insert(object_id.clone())
        {
            continue;
        }
        operations.push(operation.clone());
    }
    operations
}

/// Projection mark of one object of the investigation overlay.
#[derive(Clone, Debug)]
pub struct StagedObject {
    pub object: MapEditObject,
    /// Change set version whose replacement is shown (the last in a chain).
    pub change_set_id: String,
    pub version: i64,
    /// Every staged version that creates or edits this object.
    pub change_sets: Vec<ChangeSetKey>,
    /// Accepted revision this object replaces; `None` for staged creations.
    pub replaces_revision: Option<String>,
}

/// An accepted object edited by several staged versions that do not depend
/// on each other: the projection keeps the accepted version.
#[derive(Clone, Debug)]
pub struct StagedConflict {
    pub object_id: String,
    pub change_sets: Vec<ChangeSetKey>,
}

#[derive(Clone, Debug, Default)]
pub struct InvestigationOverlay {
    pub objects: Vec<StagedObject>,
    pub conflicts: Vec<StagedConflict>,
}

fn depends_transitively(
    index: &BTreeMap<ChangeSetKey, &StagedChangeSet>,
    from: &ChangeSetKey,
    to: &ChangeSetKey,
) -> bool {
    let mut stack = vec![from.clone()];
    let mut seen = BTreeSet::new();
    while let Some(key) = stack.pop() {
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some(set) = index.get(&key) {
            for dep in &set.depends_on {
                if dep == to {
                    return true;
                }
                stack.push(dep.clone());
            }
        }
    }
    false
}

/// Read-only `scope=investigation` overlay: every `staged` version is planned
/// against the accepted head plus its staged dependency closure, and the
/// objects it changes are merged. A version whose plan is not ready (its
/// state is stale) contributes nothing. The accepted head is never modified.
pub fn project_investigation(
    snapshot: &MapEditSnapshot,
    sets: &[StagedChangeSet],
    actor: &ActorRef,
) -> InvestigationOverlay {
    let staged: Vec<&StagedChangeSet> = sets.iter().filter(|s| s.state == "staged").collect();
    let Ok(ordered) = topological_order(&staged) else {
        return InvestigationOverlay::default();
    };
    let index: BTreeMap<ChangeSetKey, &StagedChangeSet> =
        sets.iter().map(|set| (set.key(), set)).collect();
    let accepted: BTreeMap<&str, &MapEditObject> = snapshot
        .objects
        .iter()
        .map(|object| (object.object_id.as_str(), object))
        .collect();
    // object id -> [(change set, object)] in topological order
    let mut contributions = BTreeMap::<String, Vec<(ChangeSetKey, MapEditObject)>>::new();
    for set in ordered {
        let Ok(closure) = staged_dependency_closure(sets, &[set.key()]) else {
            continue;
        };
        let operations = combined_operations(closure.iter().copied(), &set.operations);
        let own: BTreeSet<&str> = set
            .operations
            .iter()
            .map(UserChangeOperation::object_id)
            .collect();
        let request = UserChangeRequest {
            request_id: format!(
                "mathscape:investigation-projection/{}",
                crate::map_edit::edit_fingerprint(&(&set.change_set_id, set.version))
                    .trim_start_matches("sha256:")
            ),
            base_commit: snapshot.base_commit.clone(),
            reason: "investigation projection".into(),
            operations,
        };
        let planned = plan_map_change_over_head(snapshot, &request, actor);
        if planned.impact.status != mathscape_domain::ImpactPlanStatus::Ready {
            continue;
        }
        for object in planned.changed_objects {
            if own.contains(object.object_id.as_str()) {
                contributions
                    .entry(object.object_id.clone())
                    .or_default()
                    .push((set.key(), object));
            }
        }
    }
    let mut overlay = InvestigationOverlay::default();
    for (object_id, items) in contributions {
        let keys: Vec<ChangeSetKey> = items.iter().map(|(key, _)| key.clone()).collect();
        let existing = accepted.get(object_id.as_str());
        let chained = keys
            .windows(2)
            .all(|pair| depends_transitively(&index, &pair[1], &pair[0]));
        let identical = items
            .windows(2)
            .all(|pair| pair[0].1.payload == pair[1].1.payload);
        if existing.is_some() && !chained && !identical {
            overlay.conflicts.push(StagedConflict {
                object_id,
                change_sets: keys,
            });
            continue;
        }
        let (key, object) = if chained {
            items.last().cloned().expect("nonempty")
        } else {
            items.first().cloned().expect("nonempty")
        };
        overlay.objects.push(StagedObject {
            object,
            change_set_id: key.0,
            version: key.1,
            change_sets: keys,
            replaces_revision: existing.map(|object| object.revision.clone()),
        });
    }
    overlay
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(id: &str, state: &str, deps: &[&str]) -> StagedChangeSet {
        StagedChangeSet {
            change_set_id: id.into(),
            version: 1,
            state: state.into(),
            operations: vec![],
            depends_on: deps.iter().map(|dep| ((*dep).to_owned(), 1)).collect(),
        }
    }

    fn ids(sets: &[&StagedChangeSet]) -> Vec<String> {
        sets.iter().map(|set| set.change_set_id.clone()).collect()
    }

    #[test]
    fn topological_order_depends_only_on_the_graph() {
        // z ← y ← x, plus an independent b: dependencies first, ties by id.
        let all = [
            set("x", "staged", &["y"]),
            set("b", "staged", &[]),
            set("y", "staged", &["z"]),
            set("z", "staged", &[]),
        ];
        let forward: Vec<&StagedChangeSet> = all.iter().collect();
        let mut backward = forward.clone();
        backward.reverse();
        let expected = vec!["b", "z", "y", "x"];
        assert_eq!(ids(&topological_order(&forward).unwrap()), expected);
        assert_eq!(ids(&topological_order(&backward).unwrap()), expected);
        let cycle = [set("p", "staged", &["q"]), set("q", "staged", &["p"])];
        let refs: Vec<&StagedChangeSet> = cycle.iter().collect();
        assert!(matches!(
            topological_order(&refs),
            Err(ClosureError::Cycle(_))
        ));
    }

    #[test]
    fn closure_is_transitive_and_skips_accepted_dependencies() {
        let all = [
            set("a", "accepted", &[]),
            set("b", "staged", &["a"]),
            set("c", "staged", &["b"]),
            set("d", "staged", &["c"]),
        ];
        let closure = staged_dependency_closure(&all, &[("d".into(), 1)]).unwrap();
        assert_eq!(ids(&closure), vec!["b", "c"]);
        let blocked = [set("a", "blocked", &[]), set("b", "staged", &["a"])];
        assert_eq!(
            staged_dependency_closure(&blocked, &[("b".into(), 1)])
                .unwrap_err()
                .code(),
            "dependency_blocked"
        );
    }

    #[test]
    fn a_selection_must_be_closed_under_pending_dependencies() {
        let all = [
            set("a", "staged", &[]),
            set("b", "staged", &["a"]),
            set("c", "staged", &["b"]),
        ];
        assert_eq!(
            ordered_selection(&all, &[("c".into(), 1), ("b".into(), 1)])
                .unwrap_err()
                .code(),
            "dependency_not_accepted"
        );
        let ordered =
            ordered_selection(&all, &[("c".into(), 1), ("a".into(), 1), ("b".into(), 1)]).unwrap();
        assert_eq!(ids(&ordered), vec!["a", "b", "c"]);
    }
}
