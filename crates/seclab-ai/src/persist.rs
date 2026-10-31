//! Persisting the assistant settings — provider/model/sampling, the endpoint override,
//! and the privacy policy — to a `settings.ron` file so the user's choices survive a
//! restart (backlog 000108/000110/000112).
//!
//! **Keys are never written here.** An [`AssistantSettings`] holds no key material — it
//! reads key *presence* from a [`KeyStore`](crate::KeyStore) live and the key itself stays
//! in the OS keychain — so the serialized file carries only the non-secret choices. The
//! `the_saved_file_never_contains_the_api_key` test pins that guarantee.
//!
//! The caller supplies the directory (the apps pass `seclab_editor::user_modules_dir()`, so the
//! file sits beside the user's saved modules). Taking the path as an argument keeps this
//! crate free of platform-path logic and of any dependency on `seclab-editor`.

use std::path::{Path, PathBuf};

use crate::settings::AssistantSettings;

/// The settings file's basename, written inside the directory the caller supplies.
pub const SETTINGS_FILE: &str = "settings.ron";

/// Why reading or writing the settings file failed.
#[derive(Debug)]
pub enum SettingsIoError {
    /// A filesystem operation failed (creating the directory, reading, or writing).
    Io(std::io::Error),
    /// The file exists but its contents are not valid settings RON.
    Parse(String),
    /// The settings could not be serialized to RON (should not happen for this shape).
    Serialize(String),
}

impl std::fmt::Display for SettingsIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsIoError::Io(e) => write!(f, "settings file I/O error: {e}"),
            SettingsIoError::Parse(e) => write!(f, "settings file is not valid RON: {e}"),
            SettingsIoError::Serialize(e) => write!(f, "could not serialize settings: {e}"),
        }
    }
}

impl std::error::Error for SettingsIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SettingsIoError::Io(e) => Some(e),
            SettingsIoError::Parse(_) | SettingsIoError::Serialize(_) => None,
        }
    }
}

/// The full path of the settings file inside `dir`.
#[must_use]
pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

/// Writes `settings` to `dir/settings.ron`, creating `dir` (and any missing parents) if
/// needed and overwriting any existing file. No key material is written — see the module
/// docs.
///
/// # Errors
/// [`SettingsIoError::Io`] if the directory cannot be created or the file written, or
/// [`SettingsIoError::Serialize`] if the settings cannot be rendered as RON.
pub fn save_settings(dir: &Path, settings: &AssistantSettings) -> Result<(), SettingsIoError> {
    std::fs::create_dir_all(dir).map_err(SettingsIoError::Io)?;
    let text = ron::ser::to_string_pretty(settings, ron::ser::PrettyConfig::default())
        .map_err(|e| SettingsIoError::Serialize(e.to_string()))?;
    std::fs::write(settings_path(dir), text).map_err(SettingsIoError::Io)
}

/// Loads the settings from `dir/settings.ron`.
///
/// A **missing** file is not an error: the first run has nothing saved yet, so this returns
/// [`AssistantSettings::default`]. A file that is present but unparseable *is* an error, so a
/// corrupt file is surfaced to the user rather than silently reset to defaults.
///
/// # Errors
/// [`SettingsIoError::Io`] if the file exists but cannot be read, or
/// [`SettingsIoError::Parse`] if its contents are not valid settings RON.
pub fn load_settings(dir: &Path) -> Result<AssistantSettings, SettingsIoError> {
    let path = settings_path(dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AssistantSettings::default())
        }
        Err(e) => return Err(SettingsIoError::Io(e)),
    };
    ron::from_str(&text).map_err(|e| SettingsIoError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::{KeyStore, MemoryKeyStore};
    use crate::provider::ProviderKind;

    /// A fresh, unique temp directory for one test, cleaned on the way in. The caller owns
    /// cleanup on the way out. Names are per-test so parallel tests never collide.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("seclab-ai-settings-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Non-default settings touching every persisted field, so a round trip proves each one
    /// survives rather than coincidentally matching the default.
    fn edited_settings() -> AssistantSettings {
        let mut s = AssistantSettings::new();
        s.select_provider(ProviderKind::OpenAiCompatible);
        s.set_model("llama-3.1-70b");
        s.set_max_tokens(8192);
        s.set_temperature(0.3);
        s.set_endpoint("http://localhost:11434/v1/chat/completions");
        s.set_consent(true);
        s.set_offline(true);
        s
    }

    #[test]
    fn save_then_load_round_trips_every_field() {
        let dir = temp_dir("round-trip");
        let settings = edited_settings();

        save_settings(&dir, &settings).expect("save");
        let loaded = load_settings(&dir).expect("load");

        assert_eq!(
            loaded, settings,
            "every non-key field survives the round trip"
        );
        // Spot-check the individual accessors so a future field is not dropped silently.
        assert_eq!(loaded.provider(), ProviderKind::OpenAiCompatible);
        assert_eq!(loaded.model(), "llama-3.1-70b");
        assert_eq!(loaded.max_tokens(), 8192);
        assert!((loaded.temperature() - 0.3).abs() < f32::EPSILON);
        assert_eq!(
            loaded.endpoint(),
            Some("http://localhost:11434/v1/chat/completions")
        );
        assert!(loaded.consent());
        assert!(loaded.offline());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_from_an_empty_dir_yields_the_defaults() {
        // First run: no file yet. That is normal, not an error — the deny-by-default
        // settings come back.
        let dir = temp_dir("first-run");
        let loaded = load_settings(&dir).expect("missing file is not an error");
        assert_eq!(loaded, AssistantSettings::default());
        assert_eq!(loaded.provider(), ProviderKind::Anthropic);
        assert!(!loaded.consent(), "defaults still deny sending");
        // Nothing was created by a read.
        assert!(!settings_path(&dir).exists());
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_a_silent_reset() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(settings_path(&dir), "this is not ron {{{").unwrap();

        let err = load_settings(&dir).expect_err("a corrupt file must be surfaced");
        assert!(matches!(err, SettingsIoError::Parse(_)), "got {err:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_saved_file_never_contains_the_api_key() {
        // The key lives in the keystore, never on the settings struct — so even with a key
        // configured, the on-disk file must not contain it anywhere.
        let dir = temp_dir("no-key-leak");
        let mut store = MemoryKeyStore::new();
        let secret = "sk-super-secret-value-7f3a9c";
        let settings = edited_settings();
        store
            .set_key(settings.provider(), secret)
            .expect("store the key");

        save_settings(&dir, &settings).expect("save");
        let on_disk = std::fs::read_to_string(settings_path(&dir)).expect("read back");

        assert!(
            !on_disk.contains(secret),
            "the API key must never be written to settings.ron, got:\n{on_disk}"
        );
        // Sanity: the file did capture the non-secret choices.
        assert!(on_disk.contains("llama-3.1-70b"), "model is persisted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_creates_missing_parent_directories() {
        let base = temp_dir("mkdir");
        // A nested path whose parents do not exist yet.
        let dir = base.join("SecLab").join("modules");
        save_settings(&dir, &AssistantSettings::new()).expect("save creates the tree");
        assert!(settings_path(&dir).exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_file_is_ron_and_omits_an_unset_endpoint() {
        let dir = temp_dir("ron-shape");
        // Default settings leave the endpoint unset.
        save_settings(&dir, &AssistantSettings::new()).expect("save");
        let on_disk = std::fs::read_to_string(settings_path(&dir)).expect("read");

        // RON for a struct names its fields; JSON would use quotes around keys.
        assert!(
            on_disk.contains("config"),
            "named fields (RON), got:\n{on_disk}"
        );
        assert!(on_disk.contains("privacy"));
        assert!(
            !on_disk.contains("endpoint"),
            "an unset endpoint is skipped, got:\n{on_disk}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
