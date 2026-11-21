//! 「解析这几页」: the canonical reader's gap card asks for a few missing
//! pages of the view it shows.
//!
//! This is the same ensure path as `material.read`
//! ([`mathscape_mcp::ensure_canonical_pages`]): gap-only parsing, one live
//! lease per PDF, one Marker per workspace, an idempotent receipt and a scoped
//! registry sync. Two things differ for a person at the reader: coverage is
//! judged against the view on screen, and the rolling page budget that guards
//! against an agent reading a whole book does not apply
//! ([`EnsurePolicy::desktop`]). Marker is located the way the ensure probe
//! does (environment, managed venv, `PATH`); the executable this service was
//! opened with wins.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use mathscape_agent::{CommandOutput, CommandRunner, SystemCommandRunner};
use mathscape_domain::{InclusivePageRange, StableId};
use mathscape_mcp::{EnsurePolicy, EnsureReport, McpWorkspace, ensure_canonical_pages};
use mathscape_store_sqlite::{CanonicalEnsureLease, SqliteArtifactStore, unix_millis_now};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{DesktopService, DesktopState, MARKER_EXECUTABLE_ENVIRONMENT_VARIABLE};

const LEASE_OWNER: &str = "mathscape-desktop-ensure";

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ParseCanonicalPagesRequest {
    canonical_id: String,
    /// One-based, inclusive, as shown on the gap card.
    first_page: u32,
    last_page: u32,
}

impl ParseCanonicalPagesRequest {
    pub fn new(canonical_id: impl Into<String>, first_page: u32, last_page: u32) -> Self {
        Self {
            canonical_id: canonical_id.into(),
            first_page,
            last_page,
        }
    }
}

/// Live parses of this workspace, whoever started them (this reader, the
/// inspector's agent, another process), with the clock they are measured on.
#[derive(Clone, Debug, Serialize)]
pub struct CanonicalParseLeases {
    pub now_ms: i64,
    pub leases: Vec<CanonicalEnsureLease>,
}

/// The Marker this service was opened with answers
/// `MATHSCAPE_MARKER_EXECUTABLE`; everything else is the real environment.
struct ConfiguredMarker {
    executable: Option<PathBuf>,
    system: SystemCommandRunner,
}

impl CommandRunner for ConfiguredMarker {
    fn run(&self, program: &Path, args: &[&str]) -> io::Result<CommandOutput> {
        self.system.run(program, args)
    }

    fn env_var(&self, key: &str) -> Option<OsString> {
        match &self.executable {
            Some(executable) if key == MARKER_EXECUTABLE_ENVIRONMENT_VARIABLE => {
                Some(executable.clone().into_os_string())
            }
            _ => self.system.env_var(key),
        }
    }

    fn is_file(&self, path: &Path) -> bool {
        self.system.is_file(path)
    }
}

impl DesktopService {
    /// Runs Marker on the pages of `request` that the view does not show yet
    /// and waits for it. Marker problems are a report, not an error.
    pub fn parse_canonical_pages(
        &self,
        request: &ParseCanonicalPagesRequest,
    ) -> Result<EnsureReport, String> {
        self.parse_canonical_pages_with(request, EnsurePolicy::desktop())
    }

    /// As [`Self::parse_canonical_pages`] with explicit waits and limits (tests).
    pub fn parse_canonical_pages_with(
        &self,
        request: &ParseCanonicalPagesRequest,
        policy: EnsurePolicy,
    ) -> Result<EnsureReport, String> {
        let canonical_id =
            StableId::parse(request.canonical_id.clone()).map_err(|e| e.to_string())?;
        let pages = InclusivePageRange::new(request.first_page, request.last_page)
            .map_err(|_| "页码范围无效：起始页不能小于 1，结束页不能小于起始页".to_owned())?;
        if self.active_parse().is_some() {
            return Err("检查器里的「解析选定页面」正在运行，请等它结束后再试".into());
        }
        let workspace = McpWorkspace::open(&self.workspace_root)
            .map_err(|e| e.to_string())?
            .with_capability_runner(Box::new(ConfiguredMarker {
                executable: self
                    .marker
                    .as_ref()
                    .map(|marker| marker.executable().to_owned()),
                system: SystemCommandRunner::default(),
            }))
            .with_ensure_policy(policy);
        ensure_canonical_pages(&workspace, &canonical_id, pages, LEASE_OWNER)
            .map_err(|error| error.message)
    }

    pub fn canonical_parse_leases(&self) -> Result<CanonicalParseLeases, String> {
        let now_ms = unix_millis_now();
        let leases = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?
            .active_canonical_ensures(now_ms)
            .map_err(|e| e.to_string())?;
        Ok(CanonicalParseLeases { now_ms, leases })
    }
}

#[tauri::command]
pub async fn parse_canonical_pages(
    state: State<'_, DesktopState>,
    request: ParseCanonicalPagesRequest,
) -> Result<EnsureReport, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.parse_canonical_pages(&request))
        .await
        .map_err(|error| format!("canonical parse worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_canonical_parse_leases(
    state: State<'_, DesktopState>,
) -> Result<CanonicalParseLeases, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.canonical_parse_leases())
        .await
        .map_err(|e| e.to_string())?
}
