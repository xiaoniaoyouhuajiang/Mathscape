//! One-time, non-destructive import of sessions from an older sandbox root
//! (docs/codex-session-persistence.md §2.3).
//!
//! Before 2026-11 the Desktop kept every sandbox session under
//! `<app_data>/sandbox`. Sessions now live inside the mother workspace so a
//! moved workspace takes them along. On start-up the Desktop *copies* the
//! legacy sessions whose checkpoint is the current mother workspace into the
//! new root. Nothing in the legacy root is changed or deleted; a session
//! that already exists in the new root (or was discarded there) is skipped.
//! Every copied session is recorded in `migrations.jsonl`.

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    CheckpointRecord, CheckpointRegistry, Result, SESSION_FILE, SESSION_ID_PREFIX, SESSIONS_DIR,
    STAGING_DIR, SandboxError, SandboxHome, SessionRecord, copy, now_rfc3339, read_json,
    write_json_atomic,
};

const MIGRATIONS_FILE: &str = "migrations.jsonl";
const MIGRATION_SCHEMA: &str = "mathscape.sandbox_migration.v0";

/// What [`SandboxHome::import_legacy_sessions`] did.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MigrationReport {
    pub legacy_root: PathBuf,
    /// Session ids copied in this run.
    pub copied: Vec<String>,
    /// `(session id, reason)` for legacy sessions of this workspace that
    /// were left alone.
    pub skipped: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct MigrationLine {
    sandbox_session_id: String,
}

/// Session ids under `legacy_root` whose checkpoint is `mother_root`.
pub fn legacy_sessions_for(legacy_root: &Path, mother_root: &Path) -> Vec<String> {
    let mother = canonical(mother_root);
    let Ok(entries) = fs::read_dir(legacy_root.join(SESSIONS_DIR)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            if !name.starts_with(SESSION_ID_PREFIX) || !entry.path().is_dir() {
                return None;
            }
            let record: SessionRecord = read_json(&entry.path().join(SESSION_FILE)).ok()?;
            (canonical(&record.checkpoint_root) == mother).then_some(name)
        })
        .collect();
    out.sort();
    out
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

impl SandboxHome {
    /// Copy the legacy sessions of `mother_root` from `legacy_root` into this
    /// home. Idempotent; never writes to `legacy_root`.
    pub fn import_legacy_sessions(
        &self,
        legacy_root: &Path,
        mother_root: &Path,
    ) -> Result<MigrationReport> {
        let mut report = MigrationReport {
            legacy_root: legacy_root.to_path_buf(),
            ..MigrationReport::default()
        };
        if !legacy_root.join(SESSIONS_DIR).is_dir() || canonical(legacy_root) == self.root {
            return Ok(report);
        }
        let mother = canonical(mother_root);
        let migrated = self.migrated_ids()?;
        let discarded: Vec<String> = self
            .read_tombstones()?
            .into_iter()
            .map(|tombstone| tombstone.sandbox_session_id)
            .collect();
        let legacy_registry: Option<CheckpointRegistry> = {
            let path = legacy_root.join(crate::CHECKPOINTS_FILE);
            path.is_file().then(|| read_json(&path)).transpose()?
        };
        for session_id in legacy_sessions_for(legacy_root, &mother) {
            let reason = if self.session_dir(&session_id).exists() {
                Some("新位置已有同名会话")
            } else if discarded.contains(&session_id) {
                Some("新位置已丢弃该会话")
            } else if migrated.contains(&session_id) {
                Some("之前已迁移过")
            } else {
                None
            };
            if let Some(reason) = reason {
                report.skipped.push((session_id, reason.to_owned()));
                continue;
            }
            let source = legacy_root.join(SESSIONS_DIR).join(&session_id);
            let staging = self.root.join(STAGING_DIR).join(&session_id);
            let copied = (|| {
                fs::create_dir_all(&staging).map_err(|e| SandboxError::io(&staging, e))?;
                copy::copy_tree(&source, &staging)?;
                let path = staging.join(SESSION_FILE);
                let mut record: SessionRecord = read_json(&path)?;
                record.checkpoint_root = mother.clone();
                record.checkpoint_id = self.adopt_checkpoint(
                    &record.checkpoint_id,
                    &mother,
                    legacy_registry.as_ref(),
                )?;
                write_json_atomic(&path, &record)?;
                let target = self.session_dir(&session_id);
                fs::rename(&staging, &target).map_err(|e| SandboxError::io(&target, e))
            })();
            match copied {
                Ok(()) => {
                    self.append_migration(&session_id, &source)?;
                    report.copied.push(session_id);
                }
                Err(error) => {
                    let _ = fs::remove_dir_all(&staging);
                    report.skipped.push((session_id, error.to_string()));
                }
            }
        }
        Ok(report)
    }

    /// Make sure `checkpoint_id` names `root` in this home's registry and
    /// return the id to record on the copied session.
    fn adopt_checkpoint(
        &self,
        checkpoint_id: &str,
        root: &Path,
        legacy: Option<&CheckpointRegistry>,
    ) -> Result<String> {
        let mut registry = self.read_registry()?;
        if let Some(existing) = registry.checkpoints.iter().find(|c| c.root == root) {
            return Ok(existing.checkpoint_id.clone());
        }
        let legacy_record = legacy
            .and_then(|registry| {
                registry
                    .checkpoints
                    .iter()
                    .find(|c| c.checkpoint_id == checkpoint_id)
            })
            .cloned();
        let taken = registry
            .checkpoints
            .iter()
            .any(|c| c.checkpoint_id == checkpoint_id);
        let record = CheckpointRecord {
            checkpoint_id: if taken {
                format!("{checkpoint_id}-ws")
            } else {
                checkpoint_id.to_owned()
            },
            label: legacy_record
                .as_ref()
                .map(|record| record.label.clone())
                .unwrap_or_else(|| checkpoint_id.to_owned()),
            root: root.to_path_buf(),
            registered_at: legacy_record
                .map(|record| record.registered_at)
                .unwrap_or_else(now_rfc3339),
        };
        let id = record.checkpoint_id.clone();
        registry.checkpoints.push(record);
        self.write_registry(&registry)?;
        Ok(id)
    }

    fn migrated_ids(&self) -> Result<Vec<String>> {
        let path = self.root.join(MIGRATIONS_FILE);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(SandboxError::io(&path, error)),
        };
        Ok(BufReader::new(file)
            .lines()
            .map_while(std::result::Result::ok)
            .filter_map(|line| serde_json::from_str::<MigrationLine>(&line).ok())
            .map(|line| line.sandbox_session_id)
            .collect())
    }

    fn append_migration(&self, session_id: &str, source: &Path) -> Result<()> {
        let path = self.root.join(MIGRATIONS_FILE);
        let line = json!({
            "schema": MIGRATION_SCHEMA,
            "sandbox_session_id": session_id,
            "from": source,
            "mode": "copy",
            "migrated_at": now_rfc3339(),
        });
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| SandboxError::io(&path, e))?;
        writeln!(file, "{line}").map_err(|e| SandboxError::io(&path, e))
    }
}
