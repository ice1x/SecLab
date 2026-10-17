//! egui layout for the assistant Settings panel (backlog 000108/000110/000112).
//!
//! A thin rendering shell over the tested [`vb_ai::AssistantSettings`] view-model: a provider
//! dropdown, model/sampling fields, the privacy toggles (offline / consent), a masked BYOK
//! key field with save/clear, and a readiness line driven by [`vb_ai::SendBlocker`]. All the
//! rules live (and are unit-tested) in `AssistantSettings`; this file only turns that state
//! into widgets. It is the visual sketch — the exact styling and where it sits in the editor
//! are for the app to finalize.
//!
//! The key field is masked and the entered value goes straight to the [`vb_ai::KeyStore`] via
//! `AssistantSettings::store_key`; it is never echoed back or held in the view-model.

use egui::Ui;

use vb_ai::{AssistantSettings, KeyStore, ProviderKind};

/// What the Settings panel asks the host to do. Field edits mutate `settings`/`keys` in place;
/// this signals the one action the host must act on itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    /// The user pressed "apply" — the host should (re)connect the live provider from the
    /// current settings and stored key.
    Apply,
    /// The user asked to fetch the local server's installed models — the host should query
    /// it (via [`vb_ai::LocalProvider::list_models`]) and fill the panel's model list so the
    /// user can pick one instead of typing it.
    FetchModels,
}

/// A short label for a provider in the dropdown.
fn provider_label(kind: ProviderKind) -> &'static str {
    kind.label()
}

/// Renders the assistant Settings panel into `ui`, driving `settings` and the key `keys`
/// store from the widgets. `key_draft` is the panel's own scratch buffer for the masked key
/// field (kept by the caller across frames). `models` is the list of model identifiers the
/// host last fetched from the local server (empty until a fetch succeeds) — rendered as
/// one-click picks for the local provider. Returns the one [`SettingsAction`] the host must
/// act on this frame, if any.
pub fn settings_panel_ui(
    ui: &mut Ui,
    settings: &mut AssistantSettings,
    keys: &mut dyn KeyStore,
    key_draft: &mut String,
    models: &[String],
) -> Option<SettingsAction> {
    let mut action = None;

    // Provider dropdown.
    ui.horizontal(|ui| {
        ui.label("provider");
        egui::ComboBox::from_id_salt("assistant-provider")
            .selected_text(provider_label(settings.provider()))
            .show_ui(ui, |ui| {
                for kind in AssistantSettings::providers() {
                    if ui
                        .selectable_label(settings.provider() == kind, provider_label(kind))
                        .clicked()
                    {
                        settings.select_provider(kind);
                    }
                }
            });
    });

    // Model identifier, with a provider-appropriate example as the hint.
    let model_hint = match settings.provider() {
        ProviderKind::Anthropic => "e.g. claude-sonnet-5",
        ProviderKind::OpenAiCompatible => "e.g. gpt-4o-mini",
        ProviderKind::Local => "e.g. qwen2.5-coder:14b",
    };
    let mut model = settings.model().to_owned();
    ui.horizontal(|ui| {
        ui.label("model");
        if ui
            .add(egui::TextEdit::singleline(&mut model).hint_text(model_hint))
            .changed()
        {
            settings.set_model(model);
        }
    });

    // Endpoint override — meaningful for the OpenAI-compatible provider (a cloud gateway) and
    // the local provider (a self-hosted server). Blank means the provider's default endpoint:
    // api.openai.com for OpenAI-compatible, Ollama's localhost:11434 for local.
    let endpoint_hint = match settings.provider() {
        ProviderKind::OpenAiCompatible => Some("default: api.openai.com"),
        ProviderKind::Local => Some("default: localhost:11434 (Ollama)"),
        ProviderKind::Anthropic => None,
    };
    if let Some(hint) = endpoint_hint {
        let mut endpoint = settings.endpoint().unwrap_or_default().to_owned();
        ui.horizontal(|ui| {
            ui.label("endpoint");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut endpoint)
                        .hint_text(hint)
                        .desired_width(240.0),
                )
                .changed()
            {
                settings.set_endpoint(endpoint);
            }
        });
        // For a local server, offer one-click presets that fill the endpoint with each known
        // backend's default URL (Ollama / LM Studio / llama.cpp).
        if settings.provider() == ProviderKind::Local {
            ui.horizontal(|ui| {
                ui.label("presets");
                for backend in vb_ai::LocalBackend::ALL {
                    if ui.button(backend.label()).clicked() {
                        settings.set_endpoint(backend.url());
                    }
                }
            });
            // Fetch the server's installed models, then offer them as one-click picks so the
            // user need not type an id like `qwen2.5-coder:14b`. The fetch itself is the
            // host's job (it owns the network provider); this only asks for it and renders
            // whatever came back.
            ui.horizontal(|ui| {
                ui.label("models");
                if ui.button("↻ fetch").clicked() {
                    action = Some(SettingsAction::FetchModels);
                }
                if models.is_empty() {
                    ui.weak("(none fetched)");
                }
            });
            if !models.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for m in models {
                        if ui
                            .selectable_label(settings.model() == m.as_str(), m)
                            .clicked()
                        {
                            settings.set_model(m.clone());
                        }
                    }
                });
            }
        }
    }

    // Sampling.
    let mut max_tokens = settings.max_tokens();
    if ui
        .add(egui::DragValue::new(&mut max_tokens).prefix("max tokens: "))
        .changed()
    {
        settings.set_max_tokens(max_tokens);
    }
    let mut temperature = settings.temperature();
    if ui
        .add(egui::Slider::new(&mut temperature, 0.0..=2.0).text("temperature"))
        .changed()
    {
        settings.set_temperature(temperature);
    }

    ui.separator();

    // Privacy policy.
    let mut offline = settings.offline();
    if ui.checkbox(&mut offline, "offline mode").changed() {
        settings.set_offline(offline);
    }
    let mut consent = settings.consent();
    if ui
        .checkbox(&mut consent, "consent to send the project")
        .changed()
    {
        settings.set_consent(consent);
    }

    ui.separator();

    // BYOK key: masked entry that goes straight to the store, plus its live presence. A local
    // provider is keyless, so the whole section is hidden for it — there is no key to manage.
    if settings.provider().requires_key() {
        let present = settings.key_present(keys);
        ui.label(if present {
            "API key: stored ✓"
        } else {
            "API key: none"
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(key_draft)
                    .password(true)
                    .hint_text("paste API key")
                    .desired_width(160.0),
            );
            let can_save = !key_draft.trim().is_empty();
            if ui
                .add_enabled(can_save, egui::Button::new("save key"))
                .clicked()
            {
                match settings.store_key(keys, key_draft) {
                    Ok(()) => {
                        key_draft.clear();
                        settings_status(ui, "key saved");
                    }
                    Err(e) => settings_status(ui, &format!("key error: {e}")),
                }
            }
            if ui
                .add_enabled(present, egui::Button::new("clear key"))
                .clicked()
            {
                let _ = settings.clear_key(keys);
                settings_status(ui, "key cleared");
            }
        });

        ui.separator();
    }

    // Readiness: the single most pressing blocker, or a ready state.
    match settings.send_blocker(keys) {
        Some(blocker) => ui.colored_label(
            egui::Color32::from_rgb(0xd0, 0x90, 0x40),
            format!("not ready: {}", blocker.label()),
        ),
        None => ui.colored_label(egui::Color32::from_rgb(0x40, 0xb0, 0x60), "ready ✓"),
    };

    if ui.button("apply / connect").clicked() {
        action = Some(SettingsAction::Apply);
    }

    action
}

/// A transient one-liner shown under the key controls the frame an action fires.
fn settings_status(ui: &mut Ui, text: &str) {
    ui.weak(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_ai::provider::ProviderConfig;
    use vb_ai::{MemoryKeyStore, PrivacySettings};

    /// Runs `settings_panel_ui` once inside a headless egui context — no window, no display.
    /// A smoke test: the frame must build without panicking. Returns whatever it returned.
    fn run_once(
        settings: &mut AssistantSettings,
        keys: &mut dyn KeyStore,
        key_draft: &mut String,
    ) -> Option<SettingsAction> {
        run_once_with_models(settings, keys, key_draft, &[])
    }

    /// Like [`run_once`] but with a fetched-model list for the local-provider picks.
    fn run_once_with_models(
        settings: &mut AssistantSettings,
        keys: &mut dyn KeyStore,
        key_draft: &mut String,
        models: &[String],
    ) -> Option<SettingsAction> {
        let ctx = egui::Context::default();
        let mut out = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = settings_panel_ui(ui, settings, keys, key_draft, models);
            });
        });
        out
    }

    #[test]
    fn a_default_blocked_settings_panel_renders_without_panicking() {
        let mut settings = AssistantSettings::new();
        let mut keys = MemoryKeyStore::new();
        let mut draft = String::new();
        // No interaction → nothing chosen, but the frame (incl. the "not ready" line) builds.
        assert!(run_once(&mut settings, &mut keys, &mut draft).is_none());
    }

    #[test]
    fn a_ready_settings_panel_with_a_stored_key_renders_without_panicking() {
        let config = ProviderConfig {
            provider: ProviderKind::OpenAiCompatible,
            model: "some-model".to_owned(),
            ..ProviderConfig::default()
        };
        let privacy = PrivacySettings {
            offline: false,
            consent_to_send: true,
        };
        let mut settings = AssistantSettings::with(config, privacy);
        let mut keys = MemoryKeyStore::new();
        settings
            .store_key(&mut keys, "sk-test")
            .expect("stores the key");
        assert!(settings.is_ready(&keys), "precondition: the panel is ready");

        // The ready branch (green "ready ✓") and the "stored ✓" key line must render cleanly.
        let mut draft = String::new();
        assert!(run_once(&mut settings, &mut keys, &mut draft).is_none());
    }

    #[test]
    fn the_openai_endpoint_field_renders_for_a_compatible_provider() {
        let mut settings = AssistantSettings::new();
        settings.select_provider(ProviderKind::OpenAiCompatible);
        settings.set_endpoint("http://localhost:11434/v1/chat/completions");
        let mut keys = MemoryKeyStore::new();
        let mut draft = String::new();
        // The endpoint row only appears for OpenAI-compatible; the frame must build with it.
        assert!(run_once(&mut settings, &mut keys, &mut draft).is_none());
        assert_eq!(
            settings.endpoint(),
            Some("http://localhost:11434/v1/chat/completions")
        );
    }

    #[test]
    fn the_local_provider_renders_keyless_and_ready_with_an_endpoint() {
        // Picking the local provider shows the endpoint row, hides the BYOK key controls, and
        // (once consented) renders the ready line — all without a key stored. The frame must
        // build cleanly through those keyless branches.
        let mut settings = AssistantSettings::new();
        settings.select_provider(ProviderKind::Local);
        settings.set_consent(true);
        settings.set_endpoint("http://localhost:11434/v1/chat/completions");
        let mut keys = MemoryKeyStore::new();
        let mut draft = String::new();

        assert!(settings.is_ready(&keys), "precondition: keyless-ready");
        assert!(run_once(&mut settings, &mut keys, &mut draft).is_none());
        assert_eq!(
            settings.endpoint(),
            Some("http://localhost:11434/v1/chat/completions")
        );
    }

    #[test]
    fn the_local_provider_renders_its_endpoint_presets() {
        // With a blank endpoint, the local provider still renders the preset buttons row (one
        // per LocalBackend) and the model hint — the frame must build through that path.
        let mut settings = AssistantSettings::new();
        settings.select_provider(ProviderKind::Local);
        let mut keys = MemoryKeyStore::new();
        let mut draft = String::new();
        assert_eq!(settings.endpoint(), None, "precondition: blank endpoint");
        assert!(run_once(&mut settings, &mut keys, &mut draft).is_none());

        // The preset values the buttons write are the backends' own default URLs.
        settings.set_endpoint(vb_ai::LocalBackend::Ollama.url());
        assert_eq!(
            settings.endpoint(),
            Some("http://localhost:11434/v1/chat/completions")
        );
    }

    #[test]
    fn the_local_provider_renders_a_fetched_model_list() {
        // With a non-empty model list, the local provider renders the one-click picks row;
        // the frame must build through that branch without panicking.
        let mut settings = AssistantSettings::new();
        settings.select_provider(ProviderKind::Local);
        let mut keys = MemoryKeyStore::new();
        let mut draft = String::new();
        let models = vec!["qwen2.5-coder:14b".to_owned(), "llama3.1:8b".to_owned()];
        assert!(run_once_with_models(&mut settings, &mut keys, &mut draft, &models).is_none());
    }

    #[test]
    fn picking_a_fetched_model_sets_it_on_the_settings() {
        // The selectable picks drive `set_model`; assert the underlying effect directly (the
        // panel is a thin shell over this call, exercised by the rendering test above).
        let mut settings = AssistantSettings::new();
        settings.select_provider(ProviderKind::Local);
        assert_ne!(settings.model(), "qwen2.5-coder:14b");
        settings.set_model("qwen2.5-coder:14b".to_owned());
        assert_eq!(settings.model(), "qwen2.5-coder:14b");
    }

    #[test]
    fn a_non_local_provider_shows_no_model_fetch_row() {
        // The fetch row is local-only: an Anthropic panel renders cleanly even when a model
        // list is (irrelevantly) supplied — the list is simply not drawn.
        let mut settings = AssistantSettings::new();
        settings.select_provider(ProviderKind::Anthropic);
        let mut keys = MemoryKeyStore::new();
        let mut draft = String::new();
        let models = vec!["should-be-ignored".to_owned()];
        assert!(run_once_with_models(&mut settings, &mut keys, &mut draft, &models).is_none());
    }

    #[test]
    fn an_offline_settings_panel_renders_the_blocked_line() {
        let mut settings = AssistantSettings::new();
        settings.set_offline(true);
        let mut keys = MemoryKeyStore::new();
        let mut draft = "unsent draft".to_owned();
        // A non-empty masked draft exercises the save-enabled path without clicking it.
        assert!(run_once(&mut settings, &mut keys, &mut draft).is_none());
    }
}
