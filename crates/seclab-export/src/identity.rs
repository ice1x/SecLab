//! Runtime plugin identity (000265) — what makes an exported patch its **own** plugin.
//!
//! A DAW tells plugins apart by their CLAP id (and, through clap-wrapper, by the VST3 class
//! id derived from it), and shows them by name. nih-plug fixes both at compile time
//! (`ClapPlugin::CLAP_ID`, `Plugin::NAME`), so every bundle built from this crate would be
//! the same "SecLab" and a second exported patch would overwrite the first in the host.
//!
//! Instead of recompiling per patch, a bundle can carry an `identity.ron` next to its
//! `patch.vbp` (`…/Contents/Resources/identity.ron`):
//!
//! ```ron
//! (name: "Acid Bass", clap_id: "com.ice1x.seclab.user.acid-bass")
//! ```
//!
//! and `seclab-plugin`'s own CLAP entry point reports that name and id instead of the built-in
//! ones. No file (or an invalid one) keeps the built-in identity, so the stock plugin and
//! every existing DAW project are unaffected.

use serde::{Deserialize, Serialize};

/// Longest accepted display name, in characters — hosts truncate long names anyway, and a
/// bound keeps a hand-edited file from producing an unusable plugin list entry.
pub const MAX_NAME_CHARS: usize = 64;
/// Longest accepted CLAP id, in bytes (ASCII only, see [`PluginIdentity::validate`]).
pub const MAX_ID_LEN: usize = 128;
/// The identity file's name inside the bundle's `Contents/Resources`.
pub const IDENTITY_FILE: &str = "identity.ron";
/// Prefix of every exported patch's CLAP id (see [`PluginIdentity::for_name`]).
pub const USER_ID_PREFIX: &str = "com.ice1x.seclab.user.";
/// Longest slug taken from a name into an exported CLAP id.
const MAX_SLUG_LEN: usize = 48;

/// 32-bit FNV-1a — a tiny, stable, dependency-free hash for deriving ids/codes from names.
/// (Stability matters: the same name must map to the same plugin on every machine/version.)
#[must_use]
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5_u32, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// The stock plugin's display name (`Plugin::NAME` in `seclab-plugin` is this constant).
pub const BUILTIN_NAME: &str = "SecLab";
/// The stock plugin's CLAP id (`ClapPlugin::CLAP_ID` in `seclab-plugin` is this constant).
pub const BUILTIN_CLAP_ID: &str = "com.ice1x.seclab";

/// The name and CLAP id a bundle presents to the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginIdentity {
    /// Display name shown in the host's plugin list.
    pub name: String,
    /// Reverse-DNS CLAP id; the VST3 class id is derived from it by clap-wrapper.
    pub clap_id: String,
}

/// Why an identity was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// The file is not a well-formed identity RON.
    Parse(String),
    /// The name is empty, too long, or contains control characters.
    BadName,
    /// The id is empty, too long, or not `[A-Za-z0-9._-]` with no leading/trailing dot.
    BadId,
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "identity is not valid RON: {e}"),
            Self::BadName => write!(
                f,
                "plugin name must be 1–{MAX_NAME_CHARS} printable characters"
            ),
            Self::BadId => write!(
                f,
                "CLAP id must be 1–{MAX_ID_LEN} of [A-Za-z0-9._-], not starting or ending with '.'"
            ),
        }
    }
}

impl std::error::Error for IdentityError {}

impl PluginIdentity {
    /// The identity compiled into the plugin — what a bundle without `identity.ron` presents.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            name: BUILTIN_NAME.to_owned(),
            clap_id: BUILTIN_CLAP_ID.to_owned(),
        }
    }

    /// The identity an export of a patch named `name` gets: the trimmed name, and a CLAP id
    /// under [`USER_ID_PREFIX`] derived from it — a lowercase ASCII slug (`Acid Bass` →
    /// `…user.acid-bass`), plus a short hash of the full name whenever the slug alone would
    /// lose information (non-ASCII letters, punctuation), so `Бас 303` and `303` differ.
    /// Deterministic: exporting the same name again updates the same plugin.
    ///
    /// # Errors
    /// [`IdentityError::BadName`] for an unusable name.
    pub fn for_name(name: &str) -> Result<Self, IdentityError> {
        let name = name.trim();
        let mut slug = String::new();
        let mut lossless = true;
        for c in name.chars() {
            if c.is_ascii_alphanumeric() {
                slug.push(c.to_ascii_lowercase());
            } else {
                if c != ' ' && c != '-' && c != '_' {
                    lossless = false;
                }
                if !slug.is_empty() && !slug.ends_with('-') {
                    slug.push('-');
                }
            }
        }
        let full = slug.trim_end_matches('-');
        if full.len() > MAX_SLUG_LEN {
            lossless = false; // truncated: two long names could otherwise collide
        }
        let mut slug: String = full.chars().take(MAX_SLUG_LEN).collect();
        slug = slug.trim_end_matches('-').to_owned();
        if !lossless || slug.is_empty() {
            let hash = format!("{:08x}", fnv1a32(name.as_bytes()));
            slug = if slug.is_empty() {
                format!("p{hash}")
            } else {
                format!("{slug}-{hash}")
            };
        }
        let identity = Self {
            name: name.to_owned(),
            clap_id: format!("{USER_ID_PREFIX}{slug}"),
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Checks the fields are safe to hand to a host (and to `CString`).
    ///
    /// # Errors
    /// [`IdentityError::BadName`] / [`IdentityError::BadId`] on the first bad field.
    pub fn validate(&self) -> Result<(), IdentityError> {
        let name = self.name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(IdentityError::BadName);
        }
        let id = &self.clap_id;
        if id.is_empty()
            || id.len() > MAX_ID_LEN
            || id.starts_with('.')
            || id.ends_with('.')
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err(IdentityError::BadId);
        }
        Ok(())
    }

    /// Parses and validates an `identity.ron` text.
    ///
    /// # Errors
    /// [`IdentityError::Parse`] on malformed RON, else the [`Self::validate`] error.
    pub fn from_ron(text: &str) -> Result<Self, IdentityError> {
        let identity: Self =
            ron::from_str(text).map_err(|e| IdentityError::Parse(e.to_string()))?;
        identity.validate()?;
        Ok(Self {
            name: identity.name.trim().to_owned(),
            clap_id: identity.clap_id,
        })
    }

    /// Serializes the identity as the `identity.ron` an export writes.
    ///
    /// # Errors
    /// The [`Self::validate`] error — an invalid identity is never written.
    pub fn to_ron(&self) -> Result<String, IdentityError> {
        self.validate()?;
        ron::to_string(self).map_err(|e| IdentityError::Parse(e.to_string()))
    }
}

/// Where a bundle keeps its identity, given the dylib's path
/// (`…/Contents/MacOS/<bin>` → `…/Contents/Resources/identity.ron`).
#[must_use]
pub fn resources_identity_path(dylib: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos_dir = dylib.parent()?;
    let contents = macos_dir.parent()?;
    (macos_dir.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.join("Resources").join(IDENTITY_FILE))
}

/// The identity of the bundle whose dylib is at `dylib`: its `identity.ron` when present and
/// valid, else the built-in one (a broken file must never stop the plugin from loading).
#[must_use]
pub fn identity_for_dylib(dylib: Option<&std::path::Path>) -> PluginIdentity {
    dylib
        .and_then(resources_identity_path)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| PluginIdentity::from_ron(&text).ok())
        .unwrap_or_else(PluginIdentity::builtin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn acid() -> PluginIdentity {
        PluginIdentity {
            name: "Acid Bass".into(),
            clap_id: "com.ice1x.seclab.user.acid-bass".into(),
        }
    }

    #[test]
    fn an_identity_round_trips_through_its_ron_file() {
        let text = acid().to_ron().unwrap();
        assert_eq!(PluginIdentity::from_ron(&text).unwrap(), acid());
    }

    #[test]
    fn the_name_is_trimmed_and_must_be_printable_and_bounded() {
        let padded = r#"(name: "  Acid Bass ", clap_id: "com.x.y")"#;
        assert_eq!(PluginIdentity::from_ron(padded).unwrap().name, "Acid Bass");
        for bad in ["", "   ", "tab\there", &"n".repeat(MAX_NAME_CHARS + 1)] {
            let id = PluginIdentity {
                name: bad.into(),
                clap_id: "com.x.y".into(),
            };
            assert_eq!(id.validate(), Err(IdentityError::BadName), "{bad:?}");
        }
        // Non-ASCII names are fine — hosts show UTF-8.
        let ru = PluginIdentity {
            name: "Кислотный бас".into(),
            clap_id: "com.x.y".into(),
        };
        assert_eq!(ru.validate(), Ok(()));
    }

    #[test]
    fn the_clap_id_is_restricted_to_reverse_dns_characters() {
        let long = "a".repeat(MAX_ID_LEN + 1);
        for bad in [
            "", ".com.x", "com.x.", "com x", "com/x", "com.\0x", "ком.x", &long,
        ] {
            let id = PluginIdentity {
                name: "N".into(),
                clap_id: bad.into(),
            };
            assert_eq!(id.validate(), Err(IdentityError::BadId), "{bad:?}");
        }
        assert_eq!(acid().validate(), Ok(()));
    }

    #[test]
    fn malformed_ron_is_a_parse_error_not_a_panic() {
        assert!(matches!(
            PluginIdentity::from_ron("(name: 1"),
            Err(IdentityError::Parse(_))
        ));
    }

    #[test]
    fn the_identity_file_sits_in_the_bundles_resources() {
        assert_eq!(
            resources_identity_path(Path::new("/P/Acid.clap/Contents/MacOS/Acid")),
            Some("/P/Acid.clap/Contents/Resources/identity.ron".into())
        );
        // A loose dylib (Linux/Windows, tests) has no bundle and so no identity file.
        assert_eq!(resources_identity_path(Path::new("/usr/lib/x.so")), None);
    }

    #[test]
    fn a_bundle_reports_its_identity_file_and_falls_back_to_builtin() {
        let root = std::env::temp_dir().join(format!("seclab-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let contents = root.join("Acid.clap").join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        let dylib = contents.join("MacOS").join("Acid");

        // No file → the built-in SecLab identity (the stock plugin is unaffected).
        assert_eq!(identity_for_dylib(Some(&dylib)), PluginIdentity::builtin());

        let file = contents.join("Resources").join(IDENTITY_FILE);
        std::fs::write(&file, acid().to_ron().unwrap()).unwrap();
        assert_eq!(identity_for_dylib(Some(&dylib)), acid());

        // A broken file must not stop the plugin from loading — it falls back.
        std::fs::write(&file, r#"(name: "", clap_id: "x")"#).unwrap();
        assert_eq!(identity_for_dylib(Some(&dylib)), PluginIdentity::builtin());
        assert_eq!(identity_for_dylib(None), PluginIdentity::builtin());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_exported_name_gets_a_stable_user_id() {
        let acid = PluginIdentity::for_name("  Acid Bass ").unwrap();
        assert_eq!(acid.name, "Acid Bass");
        assert_eq!(acid.clap_id, "com.ice1x.seclab.user.acid-bass");
        // Deterministic: the same name re-exports onto the same plugin.
        assert_eq!(PluginIdentity::for_name("Acid Bass").unwrap(), acid);
        assert_eq!(
            PluginIdentity::for_name("tb_303 - lead").unwrap().clap_id,
            "com.ice1x.seclab.user.tb-303-lead"
        );
    }

    #[test]
    fn a_lossy_slug_gets_a_hash_so_different_names_stay_different() {
        let ru = PluginIdentity::for_name("Кислотный бас").unwrap();
        assert!(
            ru.clap_id.starts_with("com.ice1x.seclab.user.p"),
            "{}",
            ru.clap_id
        );
        let mixed = PluginIdentity::for_name("Бас 303").unwrap();
        let plain = PluginIdentity::for_name("303").unwrap();
        assert_ne!(mixed.clap_id, plain.clap_id);
        assert!(mixed.clap_id.starts_with("com.ice1x.seclab.user.303-"));
        let long_a = format!("{}a", "x".repeat(60));
        let long_b = format!("{}b", "x".repeat(60));
        assert_ne!(
            PluginIdentity::for_name(&long_a).unwrap().clap_id,
            PluginIdentity::for_name(&long_b).unwrap().clap_id
        );
        assert_ne!(
            PluginIdentity::for_name("Bass!").unwrap().clap_id,
            PluginIdentity::for_name("Bass?").unwrap().clap_id
        );
        for name in [
            "Кислотный бас",
            "Бас 303",
            "Bass!",
            &"x".repeat(MAX_NAME_CHARS),
        ] {
            PluginIdentity::for_name(name).unwrap().validate().unwrap();
        }
    }

    #[test]
    fn an_unusable_export_name_is_rejected() {
        assert_eq!(PluginIdentity::for_name("   "), Err(IdentityError::BadName));
        assert_eq!(
            PluginIdentity::for_name(&"x".repeat(MAX_NAME_CHARS + 1)),
            Err(IdentityError::BadName)
        );
    }

    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        // Published FNV-1a 32-bit test vectors.
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
        assert_eq!(fnv1a32(b"foobar"), 0xbf9c_f968);
    }

    #[test]
    fn the_builtin_identity_is_the_compiled_seclab_one() {
        let builtin = PluginIdentity::builtin();
        assert_eq!(builtin.name, "SecLab");
        assert_eq!(builtin.clap_id, "com.ice1x.seclab");
        assert_eq!(builtin.validate(), Ok(()));
    }
}
