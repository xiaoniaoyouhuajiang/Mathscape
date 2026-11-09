//! Read-only discovery of the managed Lean profile root.
//!
//! [`LeanProfileStore::new`](crate::LeanProfileStore::new) creates its root,
//! which is right for the installer but wrong for "where is the profile the
//! user already built?". This module answers that question without touching
//! the filesystem, so the desktop can show the profile in the source layer and
//! hand the same root to sandbox rounds (whose workspace is a copy that has
//! lost the parent-directory lookup).

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{LEAN_PROFILE_ROOT_ENV, ManagedLeanProfileMetadata};

/// Relative location of a workspace-local profile root.
pub const WORKSPACE_PROFILE_DIR: &str = ".mathscape/lean/profiles";

/// Where a discovered profile root came from, in search order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeanProfileRootSource {
    /// `MATHSCAPE_LEAN_PROFILE_ROOT`.
    Environment,
    /// `<workspace>/.mathscape/lean/profiles`.
    Workspace,
    /// `<workspace parent>/.mathscape/lean/profiles`.
    WorkspaceParent,
    /// `<process cwd>/.mathscape/lean/profiles`.
    CurrentDirectory,
    /// `$XDG_DATA_HOME` (or `~/.local/share`) `/mathscape/lean/profiles`.
    PlatformData,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscoveredLeanProfileRoot {
    pub root: PathBuf,
    pub source: LeanProfileRootSource,
}

/// One entry under a profile root. `metadata` is `None` when `profile.json`
/// is missing or unreadable; `problem` then says why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredLeanProfile {
    pub id: String,
    pub path: PathBuf,
    pub metadata: Option<ManagedLeanProfileMetadata>,
    pub problem: Option<String>,
}

/// Finds the profile root without creating anything.
///
/// Order: explicit environment variable (returned even if the directory is
/// missing, so a misconfiguration stays visible), then the first existing of
/// workspace, workspace parent, current directory and platform data dir.
pub fn discover_profile_root(workspace: Option<&Path>) -> Option<DiscoveredLeanProfileRoot> {
    let explicit = env::var_os(LEAN_PROFILE_ROOT_ENV).map(PathBuf::from);
    let current = env::current_dir().ok();
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    discover_with(
        workspace,
        explicit,
        current.as_deref(),
        data_home.as_deref(),
    )
}

fn discover_with(
    workspace: Option<&Path>,
    explicit: Option<PathBuf>,
    current_dir: Option<&Path>,
    data_home: Option<&Path>,
) -> Option<DiscoveredLeanProfileRoot> {
    if let Some(root) = explicit.filter(|root| !root.as_os_str().is_empty()) {
        return Some(DiscoveredLeanProfileRoot {
            root,
            source: LeanProfileRootSource::Environment,
        });
    }
    let mut candidates = Vec::new();
    if let Some(workspace) = workspace {
        candidates.push((
            workspace.join(WORKSPACE_PROFILE_DIR),
            LeanProfileRootSource::Workspace,
        ));
        if let Some(parent) = workspace.parent() {
            candidates.push((
                parent.join(WORKSPACE_PROFILE_DIR),
                LeanProfileRootSource::WorkspaceParent,
            ));
        }
    }
    if let Some(current) = current_dir {
        candidates.push((
            current.join(WORKSPACE_PROFILE_DIR),
            LeanProfileRootSource::CurrentDirectory,
        ));
    }
    if let Some(data_home) = data_home {
        candidates.push((
            data_home.join("mathscape/lean/profiles"),
            LeanProfileRootSource::PlatformData,
        ));
    }
    candidates
        .into_iter()
        .find(|(root, _)| root.is_dir())
        .map(|(root, source)| DiscoveredLeanProfileRoot {
            root: fs::canonicalize(&root).unwrap_or(root),
            source,
        })
}

/// Lists the profiles under `root` (one directory each), sorted by id.
/// Reads `profile.json` only; never probes, re-fingerprints or writes.
pub fn list_profiles(root: &Path) -> std::io::Result<Vec<DiscoveredLeanProfile>> {
    let mut profiles = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if id.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let metadata_path = path.join("profile.json");
        let (metadata, problem) = match fs::read(&metadata_path) {
            Ok(bytes) => match serde_json::from_slice::<ManagedLeanProfileMetadata>(&bytes) {
                Ok(metadata) => (Some(metadata), None),
                Err(error) => (None, Some(format!("profile.json 无法解析：{error}"))),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (None, Some("缺少 profile.json（尚未登记）".to_owned()))
            }
            Err(error) => (None, Some(format!("无法读取 profile.json：{error}"))),
        };
        profiles.push(DiscoveredLeanProfile {
            id,
            path,
            metadata,
            problem,
        });
    }
    profiles.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_root_wins_even_when_missing() {
        let found = discover_with(
            None,
            Some(PathBuf::from("/nonexistent/profiles")),
            None,
            None,
        )
        .expect("explicit root");
        assert_eq!(found.source, LeanProfileRootSource::Environment);
        assert_eq!(found.root, PathBuf::from("/nonexistent/profiles"));
    }

    #[test]
    fn parent_of_workspace_is_found_without_creating_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(temp.path().join(WORKSPACE_PROFILE_DIR)).unwrap();
        let found = discover_with(Some(&workspace), None, None, None).expect("parent root");
        assert_eq!(found.source, LeanProfileRootSource::WorkspaceParent);
        assert!(!workspace.join(".mathscape").exists());
    }

    #[test]
    fn nothing_found_creates_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        assert!(discover_with(Some(temp.path()), None, None, Some(&data)).is_none());
        assert!(!data.exists());
    }

    #[test]
    fn lists_registered_and_unregistered_profiles() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("b-unregistered")).unwrap();
        fs::create_dir_all(temp.path().join("a-broken")).unwrap();
        fs::write(temp.path().join("a-broken/profile.json"), b"{").unwrap();
        fs::create_dir_all(temp.path().join(".staging")).unwrap();
        let profiles = list_profiles(temp.path()).unwrap();
        let ids: Vec<_> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["a-broken", "b-unregistered"]);
        assert!(
            profiles
                .iter()
                .all(|p| p.metadata.is_none() && p.problem.is_some())
        );
    }
}
