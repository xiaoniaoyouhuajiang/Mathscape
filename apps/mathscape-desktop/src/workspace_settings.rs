use std::time::Duration;

use mathscape_domain::DiscoveryAdmission;
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};
use serde::{Deserialize, Serialize};
use tauri::State;
use thiserror::Error;

use crate::{DesktopService, DesktopState};

pub const WORKSPACE_SETTINGS_VERSION: &str = "mathscape-workspace-settings-1";
pub const DEFAULT_DISCOVERY_TIMEOUT_SECONDS: u64 = 600;
pub const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 15;
const MIN_DISCOVERY_TIMEOUT_SECONDS: u64 = 30;
const MAX_DISCOVERY_TIMEOUT_SECONDS: u64 = 1800;
const MIN_CONNECT_TIMEOUT_SECONDS: u64 = 5;
const MAX_CONNECT_TIMEOUT_SECONDS: u64 = 60;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryAdmissionPreference {
    #[default]
    ReviewBeforePublish,
    AutomaticCandidate,
    AskEachRun,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSettings {
    version: String,
    discovery_admission: DiscoveryAdmissionPreference,
    discovery_timeout_seconds: u64,
    connect_timeout_seconds: u64,
    /// 「Loogle 常驻模式」. Older saved settings have no field: on.
    #[serde(default = "default_true")]
    loogle_resident: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            version: WORKSPACE_SETTINGS_VERSION.into(),
            discovery_admission: DiscoveryAdmissionPreference::ReviewBeforePublish,
            discovery_timeout_seconds: DEFAULT_DISCOVERY_TIMEOUT_SECONDS,
            connect_timeout_seconds: DEFAULT_CONNECT_TIMEOUT_SECONDS,
            loogle_resident: true,
        }
    }
}

impl WorkspaceSettings {
    fn validate(&self) -> Result<(), WorkspaceSettingsError> {
        if self.version != WORKSPACE_SETTINGS_VERSION {
            return Err(WorkspaceSettingsError::Invalid(format!(
                "unsupported settings version '{}'",
                self.version
            )));
        }
        if !(MIN_DISCOVERY_TIMEOUT_SECONDS..=MAX_DISCOVERY_TIMEOUT_SECONDS)
            .contains(&self.discovery_timeout_seconds)
        {
            return Err(WorkspaceSettingsError::Invalid(format!(
                "Discovery timeout must be between {MIN_DISCOVERY_TIMEOUT_SECONDS} and {MAX_DISCOVERY_TIMEOUT_SECONDS} seconds"
            )));
        }
        if !(MIN_CONNECT_TIMEOUT_SECONDS..=MAX_CONNECT_TIMEOUT_SECONDS)
            .contains(&self.connect_timeout_seconds)
        {
            return Err(WorkspaceSettingsError::Invalid(format!(
                "connect timeout must be between {MIN_CONNECT_TIMEOUT_SECONDS} and {MAX_CONNECT_TIMEOUT_SECONDS} seconds"
            )));
        }
        Ok(())
    }

    pub const fn discovery_admission(&self) -> DiscoveryAdmissionPreference {
        self.discovery_admission
    }

    pub const fn discovery_timeout_seconds(&self) -> u64 {
        self.discovery_timeout_seconds
    }

    pub const fn connect_timeout_seconds(&self) -> u64 {
        self.connect_timeout_seconds
    }

    pub const fn loogle_resident(&self) -> bool {
        self.loogle_resident
    }

    pub(crate) fn discovery_policy(
        &self,
        admission: DiscoveryAdmission,
    ) -> EffectiveDiscoveryPolicy {
        EffectiveDiscoveryPolicy {
            admission,
            request_timeout: Duration::from_secs(self.discovery_timeout_seconds),
            connect_timeout: Duration::from_secs(self.connect_timeout_seconds),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveWorkspaceSettingsRequest {
    discovery_admission: DiscoveryAdmissionPreference,
    discovery_timeout_seconds: u64,
    connect_timeout_seconds: u64,
    #[serde(default = "default_true")]
    loogle_resident: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EffectiveDiscoveryPolicy {
    pub(crate) admission: DiscoveryAdmission,
    pub(crate) request_timeout: Duration,
    pub(crate) connect_timeout: Duration,
}

#[derive(Debug, Error)]
pub enum WorkspaceSettingsError {
    #[error("Workspace settings persistence failed: {0}")]
    Store(#[from] SqliteStoreError),
    #[error("Workspace settings are invalid: {0}")]
    Invalid(String),
}

impl DesktopService {
    pub fn workspace_settings(&self) -> Result<WorkspaceSettings, WorkspaceSettingsError> {
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let settings = store
            .workspace_settings::<WorkspaceSettings>()?
            .unwrap_or_default();
        settings.validate()?;
        Ok(settings)
    }

    pub fn save_workspace_settings(
        &self,
        request: SaveWorkspaceSettingsRequest,
    ) -> Result<WorkspaceSettings, WorkspaceSettingsError> {
        let settings = WorkspaceSettings {
            version: WORKSPACE_SETTINGS_VERSION.into(),
            discovery_admission: request.discovery_admission,
            discovery_timeout_seconds: request.discovery_timeout_seconds,
            connect_timeout_seconds: request.connect_timeout_seconds,
            loogle_resident: request.loogle_resident,
        };
        settings.validate()?;
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        store.save_workspace_settings(&settings)?;
        crate::loogle_host::set_enabled(settings.loogle_resident);
        Ok(settings)
    }
}

#[tauri::command]
pub(crate) async fn get_workspace_settings(
    state: State<'_, DesktopState>,
) -> Result<WorkspaceSettings, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.workspace_settings())
        .await
        .map_err(|error| format!("Workspace settings worker failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn save_workspace_settings(
    state: State<'_, DesktopState>,
    request: SaveWorkspaceSettingsRequest,
) -> Result<WorkspaceSettings, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.save_workspace_settings(request))
        .await
        .map_err(|error| format!("Workspace settings worker failed: {error}"))?
        .map_err(|error| error.to_string())
}
