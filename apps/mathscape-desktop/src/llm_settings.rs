use std::{sync::Arc, time::Duration};

use mathscape_artifact::ArtifactPool;
use mathscape_llm::{
    CapabilitySupport, CredentialError, CredentialRef, GenaiBackend, GenaiBackendOptions,
    LLM_PROFILE_VERSION, LlmBackend, LlmCapabilities, LlmError, LlmGenerationOptions, LlmMessage,
    LlmProfile, LlmRequest, LlmResponseFormat, LlmRole, LlmTask, LlmToolChoice, WireProtocol,
};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError, StoredLlmProfile};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tauri::State;
use thiserror::Error;
use url::Url;

use crate::{DesktopService, DesktopState, workspace_settings::WorkspaceSettingsError};

const DEFAULT_PROFILE_ID: &str = "desktop-default";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LlmSettingsSnapshot {
    profiles: Vec<LlmProfileView>,
    default_profile_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LlmProfileView {
    id: String,
    display_name: Option<String>,
    provider: String,
    wire_protocol: WireProtocol,
    model: String,
    endpoint: Option<String>,
    is_default: bool,
    has_credential: bool,
}

impl From<&StoredLlmProfile> for LlmProfileView {
    fn from(stored: &StoredLlmProfile) -> Self {
        let profile = stored.profile();
        Self {
            id: profile.id.clone(),
            display_name: profile.display_name.clone(),
            provider: profile.provider.clone(),
            wire_protocol: profile.wire_protocol,
            model: profile.model.clone(),
            endpoint: profile.endpoint.as_ref().map(ToString::to_string),
            is_default: stored.is_default(),
            has_credential: profile.credential.is_some(),
        }
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SaveLlmProfileRequest {
    #[serde(default = "default_profile_id")]
    profile_id: String,
    #[serde(default)]
    display_name: Option<String>,
    provider: String,
    wire_protocol: WireProtocol,
    model: String,
    endpoint: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    clear_api_key: bool,
    #[serde(default = "default_true")]
    make_default: bool,
}

impl std::fmt::Debug for SaveLlmProfileRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SaveLlmProfileRequest")
            .field("profile_id", &self.profile_id)
            .field("provider", &self.provider)
            .field("wire_protocol", &self.wire_protocol)
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key.as_ref().map(|_| "REDACTED"))
            .field("clear_api_key", &self.clear_api_key)
            .field("make_default", &self.make_default)
            .finish()
    }
}

fn default_profile_id() -> String {
    DEFAULT_PROFILE_ID.to_string()
}

const fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LlmConnectionTest {
    profile_id: String,
    provider_model: String,
    elapsed_ms: u64,
    response_preview: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[derive(Debug, Error)]
pub enum LlmSettingsError {
    #[error("SQLite LLM profile persistence failed: {0}")]
    Store(#[from] SqliteStoreError),
    #[error("Artifact Pool read failed: {0}")]
    Artifact(#[from] mathscape_artifact::ArtifactPoolError),
    #[error("LLM credential operation failed: {0}")]
    Credential(#[from] CredentialError),
    #[error("LLM profile is invalid: {0}")]
    InvalidProfile(String),
    #[error("LLM profile '{0}' does not exist")]
    UnknownProfile(String),
    #[error("no default LLM profile is configured")]
    NoDefaultProfile,
    #[error("LLM connection test failed: {0}")]
    Llm(#[from] LlmError),
    #[error(transparent)]
    WorkspaceSettings(#[from] WorkspaceSettingsError),
}

impl DesktopService {
    pub fn set_default_llm_profile(
        &self,
        id: &str,
    ) -> Result<LlmSettingsSnapshot, LlmSettingsError> {
        let mut store = self.llm_store()?;
        let profile = store
            .llm_profile(id)?
            .ok_or_else(|| LlmSettingsError::UnknownProfile(id.into()))?;
        store.upsert_llm_profile(profile.profile(), true)?;
        self.llm_settings()
    }
    pub fn llm_settings(&self) -> Result<LlmSettingsSnapshot, LlmSettingsError> {
        let store = self.llm_store()?;
        let stored = store.list_llm_profiles()?;
        let default_profile_id = stored
            .iter()
            .find(|profile| profile.is_default())
            .map(|profile| profile.profile().id.clone());
        Ok(LlmSettingsSnapshot {
            profiles: stored.iter().map(LlmProfileView::from).collect(),
            default_profile_id,
        })
    }

    pub fn save_llm_profile(
        &self,
        request: SaveLlmProfileRequest,
    ) -> Result<LlmSettingsSnapshot, LlmSettingsError> {
        let profile_id = request.profile_id.trim().to_string();
        if request.clear_api_key
            && request
                .api_key
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(LlmSettingsError::InvalidProfile(
                "cannot replace and clear the API key in one request".to_string(),
            ));
        }

        let mut store = self.llm_store()?;
        let existing = store.llm_profile(&profile_id)?;
        let old_credential = existing
            .as_ref()
            .and_then(|stored| stored.profile().credential.clone());
        let replacement_key = request
            .api_key
            .filter(|value| !value.trim().is_empty())
            .map(SecretString::from);
        let credential = if request.clear_api_key {
            None
        } else if replacement_key.is_some() {
            Some(self.credential_reference(&profile_id)?)
        } else {
            old_credential.clone()
        };
        let profile = LlmProfile {
            display_name: request.display_name,
            version: LLM_PROFILE_VERSION.to_string(),
            id: profile_id,
            provider: request.provider.trim().to_ascii_lowercase(),
            wire_protocol: request.wire_protocol,
            model: request.model.trim().to_string(),
            endpoint: parse_endpoint(request.endpoint.as_deref())?,
            credential,
            capabilities: LlmCapabilities {
                structured_output: CapabilitySupport::Unknown,
                tools: CapabilitySupport::Unknown,
                streaming: CapabilitySupport::Unknown,
                reasoning: CapabilitySupport::Unknown,
                usage: CapabilitySupport::Unknown,
            },
        };
        profile
            .validate()
            .map_err(|error| LlmSettingsError::InvalidProfile(error.to_string()))?;
        if let (Some(reference), Some(secret)) = (&profile.credential, &replacement_key) {
            self.credentials.store(reference, secret)?;
        }
        store.upsert_llm_profile(&profile, request.make_default)?;
        if request.clear_api_key
            && let Some(reference) = old_credential
        {
            self.credentials.delete(&reference)?;
        }
        self.llm_settings()
    }

    pub async fn test_llm_profile(
        &self,
        profile_id: Option<&str>,
    ) -> Result<LlmConnectionTest, LlmSettingsError> {
        let profile = self.selected_llm_profile(profile_id)?;
        let workspace_settings = self.workspace_settings()?;
        let credentials: Arc<dyn mathscape_llm::CredentialResolver> = self.credentials.clone();
        let backend = GenaiBackend::new(
            profile.clone(),
            credentials,
            GenaiBackendOptions {
                timeout: Duration::from_secs(45),
                connect_timeout: Duration::from_secs(workspace_settings.connect_timeout_seconds()),
            },
        )?;
        let response = backend
            .execute(LlmRequest {
                task: LlmTask {
                    kind: "connection_test".to_string(),
                    version: "1".to_string(),
                    input_fingerprint: None,
                },
                messages: vec![
                    LlmMessage::text(
                        LlmRole::System,
                        "This is a connectivity test. Reply briefly with MATHSCAPE_OK.",
                    ),
                    LlmMessage::text(LlmRole::User, "Confirm the connection."),
                ],
                tools: Vec::new(),
                tool_choice: LlmToolChoice::None,
                response_format: LlmResponseFormat::Text,
                options: LlmGenerationOptions {
                    max_output_tokens: Some(128),
                    ..LlmGenerationOptions::default()
                },
            })
            .await?;
        let response_preview = response
            .text()
            .unwrap_or_else(|| "模型返回了工具调用，但没有文本。".to_string())
            .chars()
            .take(240)
            .collect();
        Ok(LlmConnectionTest {
            profile_id: profile.id,
            provider_model: response.target.provider_model,
            elapsed_ms: response.elapsed_ms,
            response_preview,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }

    pub(crate) fn selected_llm_profile(
        &self,
        profile_id: Option<&str>,
    ) -> Result<LlmProfile, LlmSettingsError> {
        let store = self.llm_store()?;
        let stored = match profile_id {
            Some(profile_id) => store
                .llm_profile(profile_id)?
                .ok_or_else(|| LlmSettingsError::UnknownProfile(profile_id.to_string()))?,
            None => store
                .default_llm_profile()?
                .ok_or(LlmSettingsError::NoDefaultProfile)?,
        };
        Ok(stored.profile().clone())
    }

    fn llm_store(&self) -> Result<SqliteArtifactStore, SqliteStoreError> {
        SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
    }

    fn credential_reference(&self, profile_id: &str) -> Result<CredentialRef, LlmSettingsError> {
        let pool = ArtifactPool::open(&self.workspace_root)?;
        CredentialRef::new(format!(
            "{}:{profile_id}",
            pool.workspace().workspace_id().as_str()
        ))
        .map_err(LlmSettingsError::Credential)
    }
}

fn parse_endpoint(value: Option<&str>) -> Result<Option<Url>, LlmSettingsError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let normalized = if value.ends_with('/') {
        value.to_string()
    } else {
        format!("{value}/")
    };
    Url::parse(&normalized)
        .map(Some)
        .map_err(|error| LlmSettingsError::InvalidProfile(format!("endpoint is invalid: {error}")))
}

#[tauri::command]
pub(crate) async fn set_default_llm_profile(
    state: State<'_, DesktopState>,
    profile_id: String,
) -> Result<LlmSettingsSnapshot, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.set_default_llm_profile(&profile_id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn get_llm_settings(
    state: State<'_, DesktopState>,
) -> Result<LlmSettingsSnapshot, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.llm_settings())
        .await
        .map_err(|error| format!("LLM settings worker failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn save_llm_profile(
    state: State<'_, DesktopState>,
    request: SaveLlmProfileRequest,
) -> Result<LlmSettingsSnapshot, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.save_llm_profile(request))
        .await
        .map_err(|error| format!("LLM settings worker failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn test_llm_profile(
    state: State<'_, DesktopState>,
    profile_id: Option<String>,
) -> Result<LlmConnectionTest, String> {
    state
        .service
        .test_llm_profile(profile_id.as_deref())
        .await
        .map_err(|error| error.to_string())
}
