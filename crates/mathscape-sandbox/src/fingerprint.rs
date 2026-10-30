//! Checkpoint fingerprint (docs/sandbox-flow.md §3.1).
//!
//! The fingerprint always covers `mathscape.sqlite3` (size, mtime and SHA-256
//! of its bytes) plus, when present, its WAL file and `workspace.toml`. The
//! `digest` is derived from paths, sizes and content hashes only, so two
//! byte-identical workspaces (for example a checkpoint and its session copy)
//! share the same digest even though copy mtimes differ.

use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{SandboxError, WORKSPACE_DATABASE};

pub const FINGERPRINT_ALGORITHM: &str = "sha256";

/// Files that participate in the fingerprint, relative to the workspace root.
/// The first entry is mandatory; the others are included when they exist.
const FINGERPRINT_FILES: [&str; 3] = [
    WORKSPACE_DATABASE,
    "mathscape.sqlite3-wal",
    "workspace.toml",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FingerprintFile {
    pub path: String,
    pub size: u64,
    pub modified_unix_ms: Option<i64>,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointFingerprint {
    pub algorithm: String,
    /// `sha256:<hex>` over `(path, size, sha256)` of every covered file.
    pub digest: String,
    pub files: Vec<FingerprintFile>,
}

impl CheckpointFingerprint {
    /// Same bytes (ignores mtimes).
    pub fn same_content(&self, other: &Self) -> bool {
        self.algorithm == other.algorithm && self.digest == other.digest
    }
}

/// Fingerprint a workspace root. Fails when `mathscape.sqlite3` is missing.
pub fn fingerprint_workspace(root: &Path) -> Result<CheckpointFingerprint, SandboxError> {
    let database = root.join(WORKSPACE_DATABASE);
    if !database.is_file() {
        return Err(SandboxError::MissingDatabase(root.to_owned()));
    }
    let mut files = Vec::new();
    for relative in FINGERPRINT_FILES {
        let path = root.join(relative);
        if !path.is_file() {
            continue;
        }
        let metadata = path.metadata().map_err(|e| SandboxError::io(&path, e))?;
        let modified_unix_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_millis()).ok());
        let sha256 = hash_file(&path).map_err(|e| SandboxError::io(&path, e))?;
        files.push(FingerprintFile {
            path: relative.to_owned(),
            size: metadata.len(),
            modified_unix_ms,
            sha256,
        });
    }
    let mut digest = Sha256::new();
    for file in &files {
        digest.update(file.path.as_bytes());
        digest.update([0]);
        digest.update(file.size.to_le_bytes());
        digest.update(file.sha256.as_bytes());
        digest.update(b"\n");
    }
    Ok(CheckpointFingerprint {
        algorithm: FINGERPRINT_ALGORITHM.to_owned(),
        digest: format!("sha256:{}", hex(&digest.finalize())),
        files,
    })
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}
