//! Harness-thread continuity for sandbox dialogues
//! (docs/codex-session-persistence.md §2.3–§2.6).
//!
//! - which harness thread a dialogue thread continues, and the per-turn
//!   context notes (first-turn skill read, moved workspace, updated
//!   tools/skill, 「自上轮以来工作区变化」);
//! - a compact digest of the session copy taken after each turn, diffed at
//!   the start of the next one;
//! - the rollout copy kept in `<session>/harness/codex/` so the thread can be
//!   restored on another machine.
//!
//! Everything here reads the session copy or writes inside the session dir;
//! nothing touches `~/.codex` (the adapter restores rollouts on demand).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mathscape_agent::{ContextNote, ThreadContinuity};
use mathscape_sandbox::HarnessThreadBinding;
use serde::{Deserialize, Serialize};

use crate::skill_pack::SkillPack;

/// Stable session preamble, sent once as first-turn context.
pub(crate) const SESSION_PREAMBLE: &str = "你在 Mathscape 沙箱探索会话里与用户对话。可用 mathscape MCP 工具查阅当前（沙箱副本）工作区里的材料、图与手稿；引用对象时尽量给出 object_id。本对话的上下文会跨轮保留：之前轮次的工具结果仍在你的上下文里，但工作区可能已被用户改动，涉及具体对象时以本轮重新调用工具的结果为准。";
/// A later dialogue thread on the same copy.
pub(crate) const LATER_THREAD_NOTE: &str = "这是本沙箱会话里新开的一条对话：工作区可能保留了更早对话留下的状态，但更早的对话内容不在本轮上下文中；以工具查到的当前状态为准，不要假设之前说过什么。";
const DELTA_SAMPLE: usize = 8;

/// What the session copy looked like after a turn.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkspaceDigest {
    /// stable id → `kind@revision`.
    pub map_objects: BTreeMap<String, String>,
    pub artifacts: u64,
    pub ink_documents: u64,
    /// change set id → (active version, state).
    pub change_sets: BTreeMap<String, (i64, String)>,
}

pub(crate) fn digest_workspace(
    workspace: &Path,
    investigation_id: &str,
) -> Result<WorkspaceDigest, String> {
    let store =
        mathscape_store_sqlite::SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3"))
            .map_err(|error| error.to_string())?;
    let mut digest = WorkspaceDigest::default();
    for object in store.map_objects().map_err(|error| error.to_string())? {
        digest.map_objects.insert(
            object.stable_id.as_str().to_owned(),
            format!("{:?}@{}", object.kind, object.revision_id.as_str()),
        );
    }
    digest.artifacts = store.artifact_count().map_err(|error| error.to_string())?;
    digest.ink_documents = store
        .list_ink_documents()
        .map_err(|error| error.to_string())?
        .len() as u64;
    for pending in store
        .list_change_set_versions(investigation_id)
        .map_err(|error| error.to_string())?
    {
        let record = pending.record;
        let entry = digest
            .change_sets
            .entry(record.change_set_id.clone())
            .or_insert((record.version, record.state.clone()));
        if record.version >= entry.0 {
            *entry = (record.version, record.state);
        }
    }
    Ok(digest)
}

fn sample(ids: &[&String]) -> String {
    let mut text = ids
        .iter()
        .take(DELTA_SAMPLE)
        .map(|id| id.as_str())
        .collect::<Vec<_>>()
        .join("、");
    if ids.len() > DELTA_SAMPLE {
        text.push_str(&format!(" 等 {} 个", ids.len()));
    }
    text
}

/// 「自上轮以来工作区变化」. Always returns a note (also when nothing
/// changed), with the reminder to trust fresh tool results.
pub(crate) fn describe_delta(before: &WorkspaceDigest, after: &WorkspaceDigest) -> String {
    let mut lines = Vec::new();
    let added: Vec<&String> = after
        .map_objects
        .keys()
        .filter(|id| !before.map_objects.contains_key(*id))
        .collect();
    let removed: Vec<&String> = before
        .map_objects
        .keys()
        .filter(|id| !after.map_objects.contains_key(*id))
        .collect();
    let changed: Vec<&String> = after
        .map_objects
        .iter()
        .filter(|(id, revision)| {
            before
                .map_objects
                .get(*id)
                .is_some_and(|old| old != *revision)
        })
        .map(|(id, _)| id)
        .collect();
    if !added.is_empty() {
        lines.push(format!(
            "- 图谱新增 {} 个对象：{}",
            added.len(),
            sample(&added)
        ));
    }
    if !changed.is_empty() {
        lines.push(format!(
            "- 图谱修改 {} 个对象：{}",
            changed.len(),
            sample(&changed)
        ));
    }
    if !removed.is_empty() {
        lines.push(format!(
            "- 图谱移除 {} 个对象：{}",
            removed.len(),
            sample(&removed)
        ));
    }
    for (id, (version, state)) in &after.change_sets {
        match before.change_sets.get(id) {
            None => lines.push(format!("- 新提案 {id}（v{version}，{state}）")),
            Some((old_version, old_state)) if old_state != state || old_version != version => {
                lines.push(format!(
                    "- 提案 {id}：v{old_version} {old_state} → v{version} {state}"
                ));
            }
            Some(_) => {}
        }
    }
    if before.artifacts != after.artifacts {
        lines.push(format!(
            "- 材料数：{} → {}",
            before.artifacts, after.artifacts
        ));
    }
    if before.ink_documents != after.ink_documents {
        lines.push(format!(
            "- 手稿数：{} → {}",
            before.ink_documents, after.ink_documents
        ));
    }
    if lines.is_empty() {
        return "自上轮以来工作区没有变化（图谱、提案、材料、手稿均未变）。涉及具体对象时仍以本轮重新调用工具的结果为准。".to_owned();
    }
    format!(
        "自上轮以来工作区变化（用户或 Mathscape 在两轮之间所做）：\n{}\n之前轮次的工具结果可能已过时；涉及这些对象时以本轮重新调用工具的结果为准。",
        lines.join("\n")
    )
}

pub(crate) fn digest_path(state_dir: &Path, dialogue_thread: &str) -> PathBuf {
    state_dir.join(format!("{dialogue_thread}.workspace.json"))
}

pub(crate) fn load_digest(state_dir: &Path, dialogue_thread: &str) -> Option<WorkspaceDigest> {
    let bytes = fs::read(digest_path(state_dir, dialogue_thread)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn save_digest(
    state_dir: &Path,
    dialogue_thread: &str,
    digest: &WorkspaceDigest,
) -> std::io::Result<()> {
    let path = digest_path(state_dir, dialogue_thread);
    let partial = path.with_extension("json.partial");
    fs::write(&partial, serde_json::to_vec_pretty(digest)?)?;
    fs::rename(&partial, &path)
}

/// Copy the harness rollout into `state_dir` under its original file name
/// (Codex finds a restored rollout by that name). Replaces only our own
/// previous copy of the same file. Returns the file name.
pub(crate) fn keep_rollout_copy(rollout: &Path, state_dir: &Path) -> std::io::Result<String> {
    let name = rollout
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "rollout has no name")
        })?
        .to_owned();
    fs::create_dir_all(state_dir)?;
    let partial = state_dir.join(format!(".{name}.partial"));
    fs::copy(rollout, &partial)?;
    fs::rename(&partial, state_dir.join(&name))?;
    Ok(name)
}

/// Inputs for one turn's context notes.
pub(crate) struct NoteInputs<'a> {
    pub continuity: ThreadContinuity,
    /// Binding from the thread's previous turn (`None` on a fresh thread).
    pub previous: Option<&'a HarnessThreadBinding>,
    pub workspace: &'a Path,
    pub skill: Option<&'a SkillPack>,
    /// The thread was rebuilt from the transcript (context fallback).
    pub rebuilt: bool,
    pub later_thread: bool,
    pub before: Option<&'a WorkspaceDigest>,
    pub now: Option<&'a WorkspaceDigest>,
}

pub(crate) fn context_notes(inputs: &NoteInputs<'_>) -> Vec<ContextNote> {
    let mut notes = Vec::new();
    let note = |key: &str, text: String| ContextNote {
        key: key.to_owned(),
        text,
    };
    if inputs.continuity == ThreadContinuity::Started {
        // The rebuilt prompt already carries the preamble and history.
        if !inputs.rebuilt {
            notes.push(note("mathscape.session", SESSION_PREAMBLE.to_owned()));
            if inputs.later_thread {
                notes.push(note("mathscape.later_thread", LATER_THREAD_NOTE.to_owned()));
            }
        }
        if let Some(skill) = inputs.skill {
            notes.push(note("mathscape.skill", skill.first_turn_note()));
        }
        return notes;
    }
    if let Some(previous) = inputs.previous {
        if previous.workspace != inputs.workspace {
            notes.push(note(
                "mathscape.workspace_moved",
                format!(
                    "工作区路径已从 {} 变为 {}（会话被移动或在另一台机器上打开）。之前轮次里出现的旧路径一律换成新路径；用工具重新确认要用到的文件。",
                    previous.workspace.display(),
                    inputs.workspace.display()
                ),
            ));
        }
        if let Some(skill) = inputs.skill
            && (previous.skill_sha256.as_deref() != Some(skill.sha256.as_str())
                || previous.tool_schema_version != skill.tool_schema_version)
        {
            notes.push(note(
                "mathscape.skill_updated",
                skill.update_note(
                    previous.skill_version.as_deref(),
                    previous.tool_schema_version.as_deref(),
                ),
            ));
        }
    }
    if let (Some(before), Some(now)) = (inputs.before, inputs.now) {
        notes.push(note(
            "mathscape.workspace_delta",
            describe_delta(before, now),
        ));
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(objects: &[(&str, &str)], sets: &[(&str, i64, &str)]) -> WorkspaceDigest {
        WorkspaceDigest {
            map_objects: objects
                .iter()
                .map(|(id, rev)| ((*id).to_owned(), (*rev).to_owned()))
                .collect(),
            artifacts: 1,
            ink_documents: 0,
            change_sets: sets
                .iter()
                .map(|(id, v, s)| ((*id).to_owned(), (*v, (*s).to_owned())))
                .collect(),
        }
    }

    #[test]
    fn delta_names_map_and_proposal_changes() {
        let before = digest(&[("n1", "a@1"), ("n2", "a@1")], &[("cs1", 1, "staged")]);
        let mut after = digest(
            &[("n1", "a@2"), ("n3", "a@1")],
            &[("cs1", 1, "accepted"), ("cs2", 1, "staged")],
        );
        after.artifacts = 2;
        let text = describe_delta(&before, &after);
        assert!(text.starts_with("自上轮以来工作区变化"), "{text}");
        assert!(text.contains("图谱新增 1 个对象：n3"));
        assert!(text.contains("图谱修改 1 个对象：n1"));
        assert!(text.contains("图谱移除 1 个对象：n2"));
        assert!(text.contains("提案 cs1：v1 staged → v1 accepted"));
        assert!(text.contains("新提案 cs2"));
        assert!(text.contains("材料数：1 → 2"));
        assert!(text.contains("以本轮重新调用工具的结果为准"));
        assert!(describe_delta(&before, &before).contains("没有变化"));
    }

    #[test]
    fn notes_follow_continuity() {
        let ws = Path::new("/tmp/ws-new");
        let fresh = context_notes(&NoteInputs {
            continuity: ThreadContinuity::Started,
            previous: None,
            workspace: ws,
            skill: None,
            rebuilt: false,
            later_thread: true,
            before: None,
            now: None,
        });
        let keys: Vec<_> = fresh.iter().map(|note| note.key.as_str()).collect();
        assert_eq!(keys, ["mathscape.session", "mathscape.later_thread"]);

        let previous = HarnessThreadBinding {
            backend: "codex".into(),
            thread_id: "t".into(),
            workspace: PathBuf::from("/tmp/ws-old"),
            ..HarnessThreadBinding::default()
        };
        let empty = WorkspaceDigest::default();
        let resumed = context_notes(&NoteInputs {
            continuity: ThreadContinuity::Resumed,
            previous: Some(&previous),
            workspace: ws,
            skill: None,
            rebuilt: false,
            later_thread: false,
            before: Some(&empty),
            now: Some(&empty),
        });
        let keys: Vec<_> = resumed.iter().map(|note| note.key.as_str()).collect();
        assert_eq!(
            keys,
            ["mathscape.workspace_moved", "mathscape.workspace_delta"]
        );
        assert!(resumed[0].text.contains("/tmp/ws-old"));
    }

    #[test]
    fn rollout_copy_keeps_the_file_name_and_replaces_only_our_copy() {
        let dir = tempfile::tempdir().unwrap();
        let rollout = dir.path().join("rollout-2026-01-01T00-00-00-abc.jsonl");
        fs::write(&rollout, b"one").unwrap();
        let state = dir.path().join("harness/codex");
        let name = keep_rollout_copy(&rollout, &state).unwrap();
        assert_eq!(name, "rollout-2026-01-01T00-00-00-abc.jsonl");
        fs::write(&rollout, b"two").unwrap();
        keep_rollout_copy(&rollout, &state).unwrap();
        assert_eq!(fs::read(state.join(&name)).unwrap(), b"two");
        assert_eq!(fs::read(&rollout).unwrap(), b"two");
        let digest = WorkspaceDigest::default();
        save_digest(&state, "d1", &digest).unwrap();
        assert_eq!(load_digest(&state, "d1"), Some(digest));
        assert_eq!(load_digest(&state, "d2"), None);
    }
}
