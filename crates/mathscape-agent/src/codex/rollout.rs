//! Locate and restore Codex rollout files (docs/codex-session-persistence.md §2.3).
//!
//! codex-cli 0.166 persists a paginated thread as
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<thread uuid>.jsonl` and can
//! resume it by thread id from *any* subdirectory of `sessions/` as long as
//! the original file name is kept. `thread/resume.path` does not work for
//! paginated threads outside `sessions/`, so a copy kept in the workspace is
//! restored under `sessions/mathscape/` before resuming by id.

use std::fs;
use std::path::{Path, PathBuf};

/// Subdirectory of `$CODEX_HOME/sessions` that receives restored copies.
pub(crate) const RESTORE_DIR: &str = "mathscape";

fn is_rollout_for(name: &str, thread_id: &str) -> bool {
    let stem = name
        .strip_suffix(".jsonl.zst")
        .or_else(|| name.strip_suffix(".jsonl"));
    name.starts_with("rollout-")
        && stem.is_some_and(|stem| stem.ends_with(&format!("-{thread_id}")))
}

/// First rollout file for `thread_id` under `<home>/sessions` (depth-first).
pub(crate) fn find_rollout(home: &Path, thread_id: &str) -> Option<PathBuf> {
    if thread_id.trim().is_empty() {
        return None;
    }
    let mut stack = vec![home.join("sessions")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `metadata` follows symlinks: a symlinked sessions subdir counts.
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| is_rollout_for(name, thread_id))
            {
                return Some(path);
            }
        }
    }
    None
}

/// Copy `copy` (a rollout kept by the caller) into
/// `<home>/sessions/mathscape/<file name>` unless a rollout for the thread
/// already exists anywhere under `sessions/`. Never overwrites a file.
/// Returns the path Codex will find.
pub(crate) fn restore_rollout(
    home: &Path,
    thread_id: &str,
    copy: &Path,
) -> std::io::Result<PathBuf> {
    if let Some(existing) = find_rollout(home, thread_id) {
        return Ok(existing);
    }
    let name = copy
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_rollout_for(name, thread_id))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{} is not a rollout file for thread {thread_id}",
                    copy.display()
                ),
            )
        })?;
    let dir = home.join("sessions").join(RESTORE_DIR);
    fs::create_dir_all(&dir)?;
    let target = dir.join(name);
    if !target.exists() {
        let partial = dir.join(format!(".{name}.partial"));
        fs::copy(copy, &partial)?;
        fs::rename(&partial, &target)?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TID: &str = "01a1c59d-fd72-7831-83e0-10f1b2afeff8";

    #[test]
    fn finds_rollouts_in_any_sessions_subdir_and_restores_without_overwrite() {
        let home = tempfile::tempdir().unwrap();
        assert!(find_rollout(home.path(), TID).is_none());
        let name = format!("rollout-2026-11-10T21-20-28-{TID}.jsonl");
        let copy_dir = tempfile::tempdir().unwrap();
        let copy = copy_dir.path().join(&name);
        fs::write(&copy, b"{\"type\":\"session_meta\"}\n").unwrap();

        let restored = restore_rollout(home.path(), TID, &copy).unwrap();
        assert_eq!(restored, home.path().join("sessions/mathscape").join(&name));
        assert_eq!(find_rollout(home.path(), TID), Some(restored.clone()));
        // A second restore keeps the existing file.
        fs::write(&copy, b"changed").unwrap();
        assert_eq!(restore_rollout(home.path(), TID, &copy).unwrap(), restored);
        assert_eq!(
            fs::read(&restored).unwrap(),
            b"{\"type\":\"session_meta\"}\n"
        );

        // Wrong file name for the thread is refused.
        let other = copy_dir.path().join("notes.jsonl");
        fs::write(&other, b"x").unwrap();
        let fresh = tempfile::tempdir().unwrap();
        assert!(restore_rollout(fresh.path(), TID, &other).is_err());
    }

    #[test]
    fn compressed_rollouts_are_found_too() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("sessions/2026/11/10");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("rollout-2026-11-10T21-20-28-{TID}.jsonl.zst"));
        fs::write(&file, b"z").unwrap();
        assert_eq!(find_rollout(home.path(), TID), Some(file));
    }
}
