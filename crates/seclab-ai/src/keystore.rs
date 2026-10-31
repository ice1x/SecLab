//! BYOK key storage: where a user's LLM provider API keys live.
//!
//! Keys are secrets. The hard rule (see the design pillars in `docs/overview.md`) is that they never
//! touch project files, RON, or telemetry — they exist only in a [`KeyStore`], which the
//! Settings UI reads and writes. Two backends implement the trait: [`MemoryKeyStore`], an
//! ephemeral in-session store (the tested reference, and the dev/offline default), and —
//! behind the `keychain` feature — `KeychainStore`, which persists to the OS keychain
//! (macOS Security.framework). Neither backend is serializable, so a key cannot be
//! written to disk by accident.

use std::collections::HashMap;

use crate::provider::ProviderKind;

/// Why a key operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyStoreError {
    /// The key was empty or all whitespace.
    EmptyKey,
    /// The underlying secret store failed.
    Backend(String),
}

impl std::fmt::Display for KeyStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyStoreError::EmptyKey => write!(f, "the API key is empty"),
            KeyStoreError::Backend(e) => write!(f, "key store error: {e}"),
        }
    }
}

impl std::error::Error for KeyStoreError {}

/// Validates and normalizes a key before storage: trims surrounding whitespace and
/// rejects an empty result.
fn normalize_key(key: &str) -> Result<String, KeyStoreError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(KeyStoreError::EmptyKey);
    }
    Ok(trimmed.to_owned())
}

/// Storage for a per-provider API key. Implementations must never persist keys anywhere a
/// project file, log, or telemetry payload could pick them up.
pub trait KeyStore {
    /// Stores `key` for `provider`, replacing any existing one. The key is trimmed; an
    /// empty or whitespace-only key is rejected.
    ///
    /// # Errors
    /// [`KeyStoreError::EmptyKey`] for a blank key, or [`KeyStoreError::Backend`].
    fn set_key(&mut self, provider: ProviderKind, key: &str) -> Result<(), KeyStoreError>;

    /// Reads the stored key for `provider`, or `None` when none is set.
    ///
    /// # Errors
    /// [`KeyStoreError::Backend`] if the store cannot be read.
    fn get_key(&self, provider: ProviderKind) -> Result<Option<String>, KeyStoreError>;

    /// Removes any key for `provider`. Idempotent: removing a missing key succeeds.
    ///
    /// # Errors
    /// [`KeyStoreError::Backend`] if the store cannot be written.
    fn remove_key(&mut self, provider: ProviderKind) -> Result<(), KeyStoreError>;

    /// Whether a key is stored for `provider`.
    ///
    /// # Errors
    /// [`KeyStoreError::Backend`] if the store cannot be read.
    fn has_key(&self, provider: ProviderKind) -> Result<bool, KeyStoreError> {
        Ok(self.get_key(provider)?.is_some())
    }
}

/// An in-session, in-memory key store — the offline/dev default and the tested reference.
///
/// Deliberately not `Serialize`/`Deserialize`: keys live only in memory for the session.
#[derive(Default)]
pub struct MemoryKeyStore {
    keys: HashMap<ProviderKind, String>,
}

impl MemoryKeyStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl KeyStore for MemoryKeyStore {
    fn set_key(&mut self, provider: ProviderKind, key: &str) -> Result<(), KeyStoreError> {
        self.keys.insert(provider, normalize_key(key)?);
        Ok(())
    }

    fn get_key(&self, provider: ProviderKind) -> Result<Option<String>, KeyStoreError> {
        Ok(self.keys.get(&provider).cloned())
    }

    fn remove_key(&mut self, provider: ProviderKind) -> Result<(), KeyStoreError> {
        self.keys.remove(&provider);
        Ok(())
    }
}

/// The OS-keychain-backed key store (macOS Security.framework via the `keyring` crate).
///
/// Enabled by the `keychain` feature. Keys persist across sessions in the system secure
/// store; the app holds only a service name, never the key material.
#[cfg(feature = "keychain")]
pub struct KeychainStore {
    service: String,
}

#[cfg(feature = "keychain")]
impl KeychainStore {
    /// A store filing keys under `service` (e.g. `"SecLab"`), one entry per provider.
    #[must_use]
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, provider: ProviderKind) -> Result<keyring::Entry, KeyStoreError> {
        keyring::Entry::new(&self.service, provider.label())
            .map_err(|e| KeyStoreError::Backend(e.to_string()))
    }
}

#[cfg(feature = "keychain")]
impl KeyStore for KeychainStore {
    fn set_key(&mut self, provider: ProviderKind, key: &str) -> Result<(), KeyStoreError> {
        let key = normalize_key(key)?;
        self.entry(provider)?
            .set_password(&key)
            .map_err(|e| KeyStoreError::Backend(e.to_string()))
    }

    fn get_key(&self, provider: ProviderKind) -> Result<Option<String>, KeyStoreError> {
        match self.entry(provider)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyStoreError::Backend(e.to_string())),
        }
    }

    fn remove_key(&mut self, provider: ProviderKind) -> Result<(), KeyStoreError> {
        match self.entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeyStoreError::Backend(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_remove_round_trip() {
        let mut store = MemoryKeyStore::new();
        assert_eq!(store.get_key(ProviderKind::Anthropic).unwrap(), None);
        assert!(!store.has_key(ProviderKind::Anthropic).unwrap());

        store.set_key(ProviderKind::Anthropic, "sk-abc").unwrap();
        assert_eq!(
            store.get_key(ProviderKind::Anthropic).unwrap(),
            Some("sk-abc".to_owned())
        );
        assert!(store.has_key(ProviderKind::Anthropic).unwrap());

        store.remove_key(ProviderKind::Anthropic).unwrap();
        assert_eq!(store.get_key(ProviderKind::Anthropic).unwrap(), None);
        // Removing again is a no-op, not an error.
        store.remove_key(ProviderKind::Anthropic).unwrap();
    }

    #[test]
    fn keys_are_per_provider() {
        let mut store = MemoryKeyStore::new();
        store
            .set_key(ProviderKind::Anthropic, "sk-anthropic")
            .unwrap();
        store
            .set_key(ProviderKind::OpenAiCompatible, "sk-openai")
            .unwrap();
        assert_eq!(
            store.get_key(ProviderKind::Anthropic).unwrap().as_deref(),
            Some("sk-anthropic")
        );
        assert_eq!(
            store
                .get_key(ProviderKind::OpenAiCompatible)
                .unwrap()
                .as_deref(),
            Some("sk-openai")
        );
        // Removing one leaves the other.
        store.remove_key(ProviderKind::Anthropic).unwrap();
        assert!(!store.has_key(ProviderKind::Anthropic).unwrap());
        assert!(store.has_key(ProviderKind::OpenAiCompatible).unwrap());
    }

    #[test]
    fn a_new_key_replaces_the_old_one() {
        let mut store = MemoryKeyStore::new();
        store.set_key(ProviderKind::Anthropic, "first").unwrap();
        store.set_key(ProviderKind::Anthropic, "second").unwrap();
        assert_eq!(
            store.get_key(ProviderKind::Anthropic).unwrap().as_deref(),
            Some("second")
        );
    }

    #[test]
    fn keys_are_trimmed_and_blanks_rejected() {
        let mut store = MemoryKeyStore::new();
        store
            .set_key(ProviderKind::Anthropic, "  sk-padded  ")
            .unwrap();
        assert_eq!(
            store.get_key(ProviderKind::Anthropic).unwrap().as_deref(),
            Some("sk-padded"),
            "surrounding whitespace is trimmed"
        );

        assert_eq!(
            store.set_key(ProviderKind::OpenAiCompatible, "   "),
            Err(KeyStoreError::EmptyKey)
        );
        assert_eq!(
            store.set_key(ProviderKind::OpenAiCompatible, ""),
            Err(KeyStoreError::EmptyKey)
        );
        assert!(!store.has_key(ProviderKind::OpenAiCompatible).unwrap());
    }

    #[test]
    fn error_messages_do_not_leak_the_key() {
        // The empty-key error names no secret (there is none), and Display is stable.
        assert_eq!(KeyStoreError::EmptyKey.to_string(), "the API key is empty");
        assert!(KeyStoreError::Backend("locked".into())
            .to_string()
            .contains("locked"));
    }
}
