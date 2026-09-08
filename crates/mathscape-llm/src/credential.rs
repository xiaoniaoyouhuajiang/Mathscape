use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::RwLock};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialRef(String);

impl CredentialRef {
    pub fn new(value: impl Into<String>) -> Result<Self, CredentialError> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(CredentialError::InvalidReference);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CredentialError {
    #[error("credential reference is invalid")]
    InvalidReference,
    #[error("credential was not found for reference '{0}'")]
    NotFound(String),
    #[error("credential store is unavailable: {0}")]
    Unavailable(String),
}

/// Runtime-only secret lookup. Implementations may use an OS keychain, an
/// environment variable, or an in-memory test store.
pub trait CredentialResolver: Send + Sync {
    fn resolve(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError>;
}

/// Read/write credential boundary used by configuration surfaces. Secrets must
/// never be mirrored into profile persistence.
pub trait CredentialStore: CredentialResolver + std::fmt::Debug {
    fn store(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError>;

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError>;
}

/// Small runtime/test resolver. Its Debug implementation never exposes keys.
#[derive(Default)]
pub struct MemoryCredentialResolver {
    entries: RwLock<BTreeMap<CredentialRef, SecretString>>,
}

impl MemoryCredentialResolver {
    pub fn insert(
        &self,
        reference: CredentialRef,
        secret: SecretString,
    ) -> Result<(), CredentialError> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| CredentialError::Unavailable("credential lock poisoned".to_string()))?;
        entries.insert(reference, secret);
        Ok(())
    }
}

impl CredentialResolver for MemoryCredentialResolver {
    fn resolve(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        self.entries
            .read()
            .map_err(|_| CredentialError::Unavailable("credential lock poisoned".to_string()))?
            .get(reference)
            .map(|secret| SecretString::from(secret.expose_secret().to_owned()))
            .ok_or_else(|| CredentialError::NotFound(reference.as_str().to_owned()))
    }
}

impl CredentialStore for MemoryCredentialResolver {
    fn store(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.insert(
            reference.clone(),
            SecretString::from(secret.expose_secret().to_owned()),
        )
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        self.entries
            .write()
            .map_err(|_| CredentialError::Unavailable("credential lock poisoned".to_string()))?
            .remove(reference);
        Ok(())
    }
}

impl std::fmt::Debug for MemoryCredentialResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryCredentialResolver")
            .field("entries", &"REDACTED")
            .finish()
    }
}

/// OS-native credential storage: Keychain Services on macOS, Credential
/// Manager on Windows, and Secret Service on supported Linux desktops.
#[cfg(feature = "keyring")]
pub struct KeyringCredentialStore {
    service: String,
}

#[cfg(feature = "keyring")]
impl KeyringCredentialStore {
    pub fn new(service: impl Into<String>) -> Result<Self, CredentialError> {
        let service = service.into();
        if service.trim().is_empty() || service.chars().any(char::is_control) {
            return Err(CredentialError::InvalidReference);
        }
        Ok(Self { service })
    }

    fn entry(&self, reference: &CredentialRef) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(&self.service, reference.as_str())
            .map_err(|error| CredentialError::Unavailable(error.to_string()))
    }
}

#[cfg(feature = "keyring")]
impl CredentialResolver for KeyringCredentialStore {
    fn resolve(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        self.entry(reference)?
            .get_password()
            .map(SecretString::from)
            .map_err(|error| match error {
                keyring::Error::NoEntry => CredentialError::NotFound(reference.as_str().to_owned()),
                other => CredentialError::Unavailable(other.to_string()),
            })
    }
}

#[cfg(feature = "keyring")]
impl CredentialStore for KeyringCredentialStore {
    fn store(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.entry(reference)?
            .set_password(secret.expose_secret())
            .map_err(|error| CredentialError::Unavailable(error.to_string()))
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        match self.entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(CredentialError::Unavailable(error.to_string())),
        }
    }
}

#[cfg(feature = "keyring")]
impl std::fmt::Debug for KeyringCredentialStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeyringCredentialStore")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}
