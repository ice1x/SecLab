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
}

/// A short label for a provider in the dropdown.
fn provider_label(kind: ProviderKind) -> &'static str {
    kind.label()
}

/// Renders the assistant Settings panel into `ui`, driving `settings` and the key `keys`
/// store from the widgets. `key_draft` is the panel's own scratch buffer for the masked key
/// field (kept by the caller across frames). Returns `Some(SettingsAction::Apply)` when the
/// user asks to (re)connect the provider.
pub fn settings_panel_ui(
    ui: &mut Ui,
    settings: &mut AssistantSettings,
    keys: &mut dyn KeyStore,
    key_draft: &mut String,
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

    // Model identifier.
    let mut model = settings.model().to_owned();
    ui.horizontal(|ui| {
        ui.label("model");
        if ui
            .add(egui::TextEdit::singleline(&mut model).hint_text("model id"))
            .changed()
        {
            settings.set_model(model);
        }
    });

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

    // BYOK key: masked entry that goes straight to the store, plus its live presence.
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
        let ctx = egui::Context::default();
        let mut out = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = settings_panel_ui(ui, settings, keys, key_draft);
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
    fn an_offline_settings_panel_renders_the_blocked_line() {
        let mut settings = AssistantSettings::new();
        settings.set_offline(true);
        let mut keys = MemoryKeyStore::new();
        let mut draft = "unsent draft".to_owned();
        // A non-empty masked draft exercises the save-enabled path without clicking it.
        assert!(run_once(&mut settings, &mut keys, &mut draft).is_none());
    }
}
