//! The assistant **settings** as a UI-agnostic view-model (backlog 000108/000110/000112).
//!
//! A Settings screen lets the user pick a provider and model, tune sampling, manage the
//! privacy policy (offline mode, consent to send the project), and enter or clear the BYOK
//! API key. This type is that screen's state and rules with no egui in sight, so it can be
//! unit-tested and later bound to whatever renders it — the same shape as
//! [`EditSession`](crate::EditSession) and the library browser view-model.
//!
//! **Keys never live here.** The API key is a secret: it flows straight from the input field
//! to a [`KeyStore`] via [`AssistantSettings::store_key`] and is never retained on this
//! struct (which is why the struct is freely `Clone`/`Debug`-able). The view-model only ever
//! asks the store *whether* a key is present, never *what* it is.

use serde::{Deserialize, Serialize};

use crate::keystore::{KeyStore, KeyStoreError};
use crate::privacy::PrivacySettings;
use crate::provider::{ProviderConfig, ProviderKind};

/// Why the assistant cannot send a request yet — the single most pressing blocker, in the
/// order the user should resolve them. `None` from [`AssistantSettings::send_blocker`] means
/// the assistant is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendBlocker {
    /// Offline mode is on — nothing may leave the machine. Overrides every other reason.
    Offline,
    /// The user has not consented to sending the project to the provider.
    NoConsent,
    /// No API key is stored for the selected provider.
    NoKey,
    /// No model is selected — the model identifier is blank, so there is nothing to run.
    /// Reachable above all for a keyless **local** provider: it needs no key, so a blank
    /// model is the only thing between it and a request that the server would reject.
    NoModel,
}

impl SendBlocker {
    /// A short human-readable label for the UI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            SendBlocker::Offline => "offline mode is on",
            SendBlocker::NoConsent => "consent to send the project is required",
            SendBlocker::NoKey => "no API key for the selected provider",
            SendBlocker::NoModel => "no model selected",
        }
    }
}

/// The assistant Settings screen's state: the provider/model/sampling
/// [`ProviderConfig`] and the [`PrivacySettings`]. Key presence is read live from a
/// [`KeyStore`] — the key value is never stored here.
///
/// This is also the on-disk settings shape ([`crate::persist`]): it serializes cleanly
/// precisely *because* it holds no secret — only the non-key choices reach `settings.ron`.
/// `#[serde(default)]` lets a partial or older file load, each missing field taking its
/// default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantSettings {
    config: ProviderConfig,
    privacy: PrivacySettings,
    /// Base URL override for the OpenAI-compatible and local providers (`None` = the
    /// provider's default endpoint: `api.openai.com` for OpenAI-compatible, Ollama's
    /// `localhost:11434` for local). This is what lets those providers cover third-party and
    /// self-hosted servers (OpenAI's shim, Together, Groq, a gateway, or a local Ollama / LM
    /// Studio / llama.cpp on a non-standard port). Ignored for the Anthropic provider, which
    /// has a fixed endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
}

impl AssistantSettings {
    /// Fresh settings: the default provider/model and the deny-by-default privacy policy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Settings from an existing config and privacy policy (e.g. restored from disk — note
    /// neither carries a key). The endpoint override starts unset.
    #[must_use]
    pub fn with(config: ProviderConfig, privacy: PrivacySettings) -> Self {
        Self {
            config,
            privacy,
            endpoint: None,
        }
    }

    /// The custom OpenAI-compatible base URL, or `None` for the default OpenAI endpoint.
    #[must_use]
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// Sets (or with a blank value, clears) the OpenAI-compatible base URL override. The URL
    /// is trimmed; an empty or whitespace-only value resets to the default endpoint.
    pub fn set_endpoint(&mut self, url: impl Into<String>) {
        let url = url.into();
        let trimmed = url.trim();
        self.endpoint = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        };
    }

    /// The provider/model/sampling config, ready to hand to the generation loop.
    #[must_use]
    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }

    /// The privacy policy.
    #[must_use]
    pub fn privacy(&self) -> &PrivacySettings {
        &self.privacy
    }

    /// The selected provider.
    #[must_use]
    pub fn provider(&self) -> ProviderKind {
        self.config.provider
    }

    /// The selected model identifier.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// The generation token budget.
    #[must_use]
    pub fn max_tokens(&self) -> u32 {
        self.config.max_tokens
    }

    /// The sampling temperature.
    #[must_use]
    pub fn temperature(&self) -> f32 {
        self.config.temperature
    }

    /// Whether offline mode is on.
    #[must_use]
    pub fn offline(&self) -> bool {
        self.privacy.offline
    }

    /// Whether the user has consented to sending the project.
    #[must_use]
    pub fn consent(&self) -> bool {
        self.privacy.consent_to_send
    }

    /// Every provider the UI can offer, in display order.
    #[must_use]
    pub fn providers() -> [ProviderKind; 3] {
        [
            ProviderKind::Anthropic,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Local,
        ]
    }

    /// Selects `provider`. The model is left as-is — the UI sets one appropriate to the new
    /// provider (there is no single cross-provider default model).
    pub fn select_provider(&mut self, provider: ProviderKind) {
        self.config.provider = provider;
    }

    /// Sets the model identifier.
    pub fn set_model(&mut self, model: impl Into<String>) {
        self.config.model = model.into();
    }

    /// Sets the generation token budget.
    pub fn set_max_tokens(&mut self, max_tokens: u32) {
        self.config.max_tokens = max_tokens;
    }

    /// Sets the sampling temperature, clamped to the `[0.0, 2.0]` range providers accept
    /// (0 = deterministic).
    pub fn set_temperature(&mut self, temperature: f32) {
        self.config.temperature = temperature.clamp(0.0, 2.0);
    }

    /// Turns offline mode on or off.
    pub fn set_offline(&mut self, offline: bool) {
        self.privacy.offline = offline;
    }

    /// Records or withdraws the user's consent to send the project.
    pub fn set_consent(&mut self, consent: bool) {
        self.privacy.consent_to_send = consent;
    }

    /// Whether a key is stored for the **selected** provider. A store read error is treated
    /// as "no key" so the UI degrades to the safe (blocked) state.
    #[must_use]
    pub fn key_present(&self, store: &dyn KeyStore) -> bool {
        store.has_key(self.config.provider).unwrap_or(false)
    }

    /// Stores `key` for the selected provider. The value passes straight to `store` and is
    /// **not** retained on this struct.
    ///
    /// # Errors
    /// [`KeyStoreError::EmptyKey`] for a blank key, or a [`KeyStoreError::Backend`] failure.
    pub fn store_key(&self, store: &mut dyn KeyStore, key: &str) -> Result<(), KeyStoreError> {
        store.set_key(self.config.provider, key)
    }

    /// Removes any stored key for the selected provider (idempotent).
    ///
    /// # Errors
    /// [`KeyStoreError::Backend`] if the store cannot be written.
    pub fn clear_key(&self, store: &mut dyn KeyStore) -> Result<(), KeyStoreError> {
        store.remove_key(self.config.provider)
    }

    /// The single most pressing reason the assistant cannot send, or `None` when it is ready:
    /// offline mode first, then missing consent, then a missing key for the selected
    /// provider. Mirrors [`PrivacySettings::blocked_reason`]'s precedence and adds the key
    /// check the privacy layer cannot see.
    ///
    /// A **local** provider is keyless ([`ProviderKind::requires_key`]), so the key check is
    /// skipped for it — a local model is ready once online and consented *and a model is
    /// picked*. The model check comes last, after the privacy and key gates: a blank model
    /// identifier can never produce a valid request, whichever provider is selected.
    #[must_use]
    pub fn send_blocker(&self, store: &dyn KeyStore) -> Option<SendBlocker> {
        if self.privacy.offline {
            Some(SendBlocker::Offline)
        } else if !self.privacy.consent_to_send {
            Some(SendBlocker::NoConsent)
        } else if self.config.provider.requires_key() && !self.key_present(store) {
            Some(SendBlocker::NoKey)
        } else if self.config.model.trim().is_empty() {
            Some(SendBlocker::NoModel)
        } else {
            None
        }
    }

    /// Whether the assistant is ready to send with the current settings and stored key.
    #[must_use]
    pub fn is_ready(&self, store: &dyn KeyStore) -> bool {
        self.send_blocker(store).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::MemoryKeyStore;
    use crate::provider::DEFAULT_MODEL;

    #[test]
    fn defaults_pick_claude_and_deny_sending() {
        let s = AssistantSettings::new();
        assert_eq!(s.provider(), ProviderKind::Anthropic);
        assert_eq!(s.model(), DEFAULT_MODEL);
        assert!(!s.offline());
        assert!(!s.consent(), "consent is off by default");
        // With no consent (and no key), the first blocker the user must resolve is consent.
        let store = MemoryKeyStore::new();
        assert_eq!(s.send_blocker(&store), Some(SendBlocker::NoConsent));
        assert!(!s.is_ready(&store));
    }

    #[test]
    fn consent_plus_a_stored_key_makes_it_ready() {
        let mut s = AssistantSettings::new();
        let mut store = MemoryKeyStore::new();
        s.set_consent(true);
        // Consent alone is not enough — the key is still missing.
        assert_eq!(s.send_blocker(&store), Some(SendBlocker::NoKey));

        s.store_key(&mut store, "sk-secret")
            .expect("stores the key");
        assert!(s.key_present(&store));
        assert_eq!(s.send_blocker(&store), None);
        assert!(s.is_ready(&store));
    }

    #[test]
    fn offline_overrides_consent_and_key() {
        let mut s = AssistantSettings::new();
        let mut store = MemoryKeyStore::new();
        s.set_consent(true);
        s.store_key(&mut store, "sk-secret").unwrap();
        assert!(s.is_ready(&store));

        s.set_offline(true);
        assert_eq!(
            s.send_blocker(&store),
            Some(SendBlocker::Offline),
            "offline wins over an otherwise-ready state"
        );
    }

    #[test]
    fn key_presence_follows_the_selected_provider() {
        let mut s = AssistantSettings::new();
        let mut store = MemoryKeyStore::new();
        // A key for Anthropic only.
        s.store_key(&mut store, "sk-anthropic").unwrap();
        assert!(s.key_present(&store));

        s.select_provider(ProviderKind::OpenAiCompatible);
        assert_eq!(s.provider(), ProviderKind::OpenAiCompatible);
        assert!(
            !s.key_present(&store),
            "the other provider has no key of its own"
        );

        s.store_key(&mut store, "sk-openai").unwrap();
        assert!(s.key_present(&store), "now it does");
        // The original provider still has its own key.
        s.select_provider(ProviderKind::Anthropic);
        assert!(s.key_present(&store));
    }

    #[test]
    fn store_and_clear_key_delegate_to_the_store() {
        let s = AssistantSettings::new();
        let mut store = MemoryKeyStore::new();

        assert!(!s.key_present(&store));
        s.store_key(&mut store, "  sk-padded  ")
            .expect("trims and stores");
        assert!(s.key_present(&store));
        // The value went to the store, trimmed — the view-model never held it.
        assert_eq!(
            store.get_key(ProviderKind::Anthropic).unwrap().as_deref(),
            Some("sk-padded")
        );

        s.clear_key(&mut store).expect("removes it");
        assert!(!s.key_present(&store));

        // A blank key is rejected by the store's validation, surfaced verbatim.
        assert_eq!(s.store_key(&mut store, "   "), Err(KeyStoreError::EmptyKey));
    }

    #[test]
    fn setters_edit_config_and_clamp_temperature() {
        let mut s = AssistantSettings::new();
        s.set_model("claude-opus-5");
        s.set_max_tokens(8192);
        s.set_temperature(5.0);
        assert_eq!(s.model(), "claude-opus-5");
        assert_eq!(s.max_tokens(), 8192);
        assert_eq!(
            s.temperature(),
            2.0,
            "temperature clamps to the upper bound"
        );
        s.set_temperature(-1.0);
        assert_eq!(s.temperature(), 0.0, "and to the lower bound");

        // The config handed to the generation loop reflects the edits.
        assert_eq!(s.config().model, "claude-opus-5");
        assert_eq!(s.config().max_tokens, 8192);
    }

    #[test]
    fn endpoint_override_sets_trims_and_clears() {
        let mut s = AssistantSettings::new();
        assert_eq!(s.endpoint(), None, "default is the OpenAI endpoint");

        s.set_endpoint("  http://localhost:11434/v1/chat/completions  ");
        assert_eq!(
            s.endpoint(),
            Some("http://localhost:11434/v1/chat/completions"),
            "the URL is trimmed and stored"
        );

        // A blank value resets to the default endpoint.
        s.set_endpoint("   ");
        assert_eq!(s.endpoint(), None);
    }

    #[test]
    fn providers_lists_every_backend() {
        let all = AssistantSettings::providers();
        assert!(all.contains(&ProviderKind::Anthropic));
        assert!(all.contains(&ProviderKind::OpenAiCompatible));
        assert!(all.contains(&ProviderKind::Local));
    }

    #[test]
    fn a_local_provider_is_ready_without_any_key() {
        // A local server (Ollama/LM Studio/llama.cpp) is keyless: once online and consented,
        // the assistant is ready with no BYOK key stored at all.
        let mut s = AssistantSettings::new();
        s.select_provider(ProviderKind::Local);
        s.set_consent(true);
        let store = MemoryKeyStore::new();

        assert!(!s.key_present(&store), "precondition: no key is stored");
        assert_eq!(
            s.send_blocker(&store),
            None,
            "a local provider needs no key to be ready"
        );
        assert!(s.is_ready(&store));

        // Offline still wins — the explicit switch blocks even a local call.
        s.set_offline(true);
        assert_eq!(s.send_blocker(&store), Some(SendBlocker::Offline));
        // And consent is still required before it is ready.
        s.set_offline(false);
        s.set_consent(false);
        assert_eq!(s.send_blocker(&store), Some(SendBlocker::NoConsent));
    }

    #[test]
    fn a_blank_model_blocks_even_a_keyless_local_provider() {
        // The keyless local path clears offline+consent+key, so a blank model is the only
        // remaining gate — and a request with no model would be rejected by the server, so the
        // readiness line must flag it rather than claim "ready".
        let mut s = AssistantSettings::new();
        s.select_provider(ProviderKind::Local);
        s.set_consent(true);
        let store = MemoryKeyStore::new();
        // Precondition: with the default (non-empty) model it is already ready.
        assert_eq!(s.send_blocker(&store), None);

        s.set_model("   ");
        assert_eq!(
            s.send_blocker(&store),
            Some(SendBlocker::NoModel),
            "a whitespace-only model is no model"
        );
        assert!(!s.is_ready(&store));

        // Picking a real model clears it.
        s.set_model("qwen2.5-coder:14b");
        assert_eq!(s.send_blocker(&store), None);
        assert!(s.is_ready(&store));
    }

    #[test]
    fn the_model_check_comes_after_the_key_gate() {
        // For a cloud provider a missing key is reported before a blank model — the user
        // resolves the auth gate first, then the model.
        let mut s = AssistantSettings::new();
        s.select_provider(ProviderKind::OpenAiCompatible);
        s.set_consent(true);
        s.set_model("");
        let mut store = MemoryKeyStore::new();
        assert_eq!(
            s.send_blocker(&store),
            Some(SendBlocker::NoKey),
            "the key gate ranks ahead of the model gate"
        );

        s.store_key(&mut store, "sk-openai").unwrap();
        assert_eq!(
            s.send_blocker(&store),
            Some(SendBlocker::NoModel),
            "with a key stored, the blank model is the next blocker"
        );
    }

    #[test]
    fn a_cloud_provider_still_requires_its_key() {
        // The keyless exemption is local-only: the OpenAI-compatible cloud path still blocks
        // on a missing key even with consent granted.
        let mut s = AssistantSettings::new();
        s.select_provider(ProviderKind::OpenAiCompatible);
        s.set_consent(true);
        let store = MemoryKeyStore::new();
        assert_eq!(s.send_blocker(&store), Some(SendBlocker::NoKey));
    }

    #[test]
    fn with_restores_an_existing_config_and_policy() {
        let config = ProviderConfig {
            provider: ProviderKind::OpenAiCompatible,
            model: "gpt-ish".to_owned(),
            ..ProviderConfig::default()
        };
        let privacy = PrivacySettings {
            offline: false,
            consent_to_send: true,
        };
        let s = AssistantSettings::with(config, privacy);
        assert_eq!(s.provider(), ProviderKind::OpenAiCompatible);
        assert_eq!(s.model(), "gpt-ish");
        assert!(s.consent());
    }
}
