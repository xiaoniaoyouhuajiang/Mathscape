//! Full-copy isolation (T1 = v0 full copy, docs/sandbox-flow.md §3.3).

use std::{fs, path::Path};

use crate::SandboxError;

/// Recursively copy `source` into the (already created) `destination`.
///
/// * Regular files are copied byte for byte; the source is only ever read.
/// * Symlinks to files are dereferenced (the copy holds the bytes, never a
///   link back into the checkpoint, so session writes cannot leak into it).
/// * Symlinks to directories and special files are rejected rather than
///   silently skipped, so a session never holds a partial workspace.
/// * `*.sqlite3-shm` is skipped: SQLite rebuilds the shared-memory index from
///   the WAL and a copied index could disagree with the copied WAL.
pub(crate) fn copy_tree(source: &Path, destination: &Path) -> Result<u64, SandboxError> {
    let mut copied = 0_u64;
    let entries = fs::read_dir(source).map_err(|e| SandboxError::io(source, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| SandboxError::io(source, e))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let link_meta = fs::symlink_metadata(&from).map_err(|e| SandboxError::io(&from, e))?;
        let file_type = link_meta.file_type();
        if file_type.is_dir() {
            fs::create_dir(&to).map_err(|e| SandboxError::io(&to, e))?;
            copied += copy_tree(&from, &to)?;
            continue;
        }
        if is_shared_memory_index(&from) {
            continue;
        }
        let meta = if file_type.is_symlink() {
            fs::metadata(&from).map_err(|e| SandboxError::io(&from, e))?
        } else {
            link_meta
        };
        if meta.is_dir() {
            return Err(SandboxError::UnsupportedEntry {
                path: from,
                reason: "指向目录的符号链接",
            });
        }
        if !meta.is_file() {
            return Err(SandboxError::UnsupportedEntry {
                path: from,
                reason: "非普通文件",
            });
        }
        fs::copy(&from, &to).map_err(|e| SandboxError::io(&from, e))?;
        copied += 1;
    }
    Ok(copied)
}

fn is_shared_memory_index(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".sqlite3-shm"))
}
