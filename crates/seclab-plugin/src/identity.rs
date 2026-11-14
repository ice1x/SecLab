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
//! and our own CLAP entry point ([`entry`]) reports that name and id instead of the built-in
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
        use nih_plug::prelude::{ClapPlugin, Plugin};
        Self {
            name: crate::SecLab::NAME.to_owned(),
            clap_id: crate::SecLab::CLAP_ID.to_owned(),
        }
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

/// Our CLAP entry point: nih-plug's factory, but reporting the bundle's [`PluginIdentity`].
///
/// This replaces `nih_export_clap!`, whose descriptor is fixed at compile time. The plugin
/// object itself is still nih-plug's [`Wrapper`](nih_plug::wrapper::clap::Wrapper); only the
/// descriptor (the id and name the host sees) is swapped.
pub mod entry {
    use std::ffi::{c_void, CStr, CString};
    use std::os::raw::c_char;
    use std::sync::OnceLock;

    use nih_plug::wrapper::clap::{
        clap_host, clap_plugin, clap_plugin_descriptor, clap_plugin_entry, clap_plugin_factory,
        PluginDescriptor, Wrapper, CLAP_PLUGIN_FACTORY_ID, CLAP_VERSION,
    };

    use super::PluginIdentity;
    use crate::SecLab;

    /// A nih-plug descriptor with the id and name replaced. The raw descriptor's other string
    /// pointers point into `_base`'s heap buffers, and `id`/`name` into ours, so they stay
    /// valid for as long as this value lives (it lives in a `static`).
    pub struct IdentityDescriptor {
        _base: PluginDescriptor,
        id: CString,
        name: CString,
        raw: clap_plugin_descriptor,
    }

    // SAFETY: the raw pointers only reference immutable heap data owned by this same value,
    // which is never mutated after construction (the same argument nih-plug makes for
    // `PluginDescriptor`).
    unsafe impl Send for IdentityDescriptor {}
    // SAFETY: as above — shared access is read-only.
    unsafe impl Sync for IdentityDescriptor {}

    impl IdentityDescriptor {
        /// Builds the descriptor for `identity` (which must be [`PluginIdentity::validate`]d,
        /// so it has no interior NULs).
        #[must_use]
        pub fn new(identity: &PluginIdentity) -> Self {
            let base = PluginDescriptor::for_plugin::<SecLab>();
            let id = CString::new(identity.clap_id.as_str()).unwrap_or_default();
            let name = CString::new(identity.name.as_str()).unwrap_or_default();
            let mut raw = *base.clap_plugin_descriptor();
            raw.id = id.as_ptr();
            raw.name = name.as_ptr();
            Self {
                _base: base,
                id,
                name,
                raw,
            }
        }

        /// The descriptor handed to the host.
        #[must_use]
        pub fn raw(&self) -> &clap_plugin_descriptor {
            &self.raw
        }

        /// The CLAP id this descriptor reports.
        #[must_use]
        pub fn id(&self) -> &CStr {
            &self.id
        }

        /// The display name this descriptor reports.
        #[must_use]
        pub fn name(&self) -> &CStr {
            &self.name
        }
    }

    static DESCRIPTOR: OnceLock<IdentityDescriptor> = OnceLock::new();

    /// The descriptor for this bundle, resolved once from its `identity.ron`.
    pub fn descriptor() -> &'static IdentityDescriptor {
        DESCRIPTOR.get_or_init(|| {
            let dylib = crate::own_dylib_path();
            IdentityDescriptor::new(&super::identity_for_dylib(dylib.as_deref()))
        })
    }

    const FACTORY: clap_plugin_factory = clap_plugin_factory {
        get_plugin_count: Some(get_plugin_count),
        get_plugin_descriptor: Some(get_plugin_descriptor),
        create_plugin: Some(create_plugin),
    };

    pub(crate) unsafe extern "C" fn get_plugin_count(_factory: *const clap_plugin_factory) -> u32 {
        1
    }

    pub(crate) unsafe extern "C" fn get_plugin_descriptor(
        _factory: *const clap_plugin_factory,
        index: u32,
    ) -> *const clap_plugin_descriptor {
        if index == 0 {
            descriptor().raw()
        } else {
            std::ptr::null()
        }
    }

    pub(crate) unsafe extern "C" fn create_plugin(
        _factory: *const clap_plugin_factory,
        host: *const clap_host,
        plugin_id: *const c_char,
    ) -> *const clap_plugin {
        if plugin_id.is_null() || host.is_null() {
            return std::ptr::null();
        }
        let desc = descriptor();
        // SAFETY: the host passes a valid NUL-terminated id (checked non-null above).
        if unsafe { CStr::from_ptr(plugin_id) } != desc.id() {
            return std::ptr::null();
        }
        // SAFETY: `host` is the host's valid `clap_host`, as the CLAP contract requires.
        // Ownership of the Arc passes to the host and is reclaimed in `Wrapper::destroy`,
        // exactly as `nih_export_clap!` does.
        let wrapper = unsafe { Wrapper::<SecLab>::new(host) };
        let plugin = std::sync::Arc::into_raw(wrapper);
        // SAFETY: `plugin` is the live wrapper we just leaked; `clap_plugin` is its own vtable,
        // not yet handed to the host, so pointing `desc` at our 'static descriptor is the
        // only write and races nothing.
        unsafe {
            let raw = (*plugin).clap_plugin.as_ptr();
            (*raw).desc = desc.raw();
            raw
        }
    }

    extern "C" fn init(_plugin_path: *const c_char) -> bool {
        nih_plug::wrapper::setup_logger();
        true
    }

    extern "C" fn deinit() {}

    extern "C" fn get_factory(factory_id: *const c_char) -> *const c_void {
        // SAFETY: a non-null factory id from the host is a valid C string.
        if !factory_id.is_null() && unsafe { CStr::from_ptr(factory_id) } == CLAP_PLUGIN_FACTORY_ID
        {
            std::ptr::from_ref(&FACTORY).cast()
        } else {
            std::ptr::null()
        }
    }

    /// The CLAP plugin's entry point.
    #[no_mangle]
    #[used]
    pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
        clap_version: CLAP_VERSION,
        init: Some(init),
        deinit: Some(deinit),
        get_factory: Some(get_factory),
    };
}

#[cfg(test)]
mod tests {
    use super::entry::{
        create_plugin, get_plugin_count, get_plugin_descriptor, IdentityDescriptor,
    };
    use super::*;
    use std::ffi::CStr;
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
    fn the_builtin_identity_is_the_compiled_seclab_one() {
        let builtin = PluginIdentity::builtin();
        assert_eq!(builtin.name, "SecLab");
        assert_eq!(builtin.clap_id, "com.ice1x.seclab");
        assert_eq!(builtin.validate(), Ok(()));
    }

    #[test]
    fn the_descriptor_swaps_id_and_name_and_keeps_everything_else() {
        let stock = IdentityDescriptor::new(&PluginIdentity::builtin());
        let custom = IdentityDescriptor::new(&acid());
        // SAFETY (whole block): every pointer read is a NUL-terminated string owned by the
        // descriptor values above, which outlive the reads.
        unsafe {
            let (s, c) = (stock.raw(), custom.raw());
            assert_eq!(CStr::from_ptr(c.id).to_str().unwrap(), acid().clap_id);
            assert_eq!(CStr::from_ptr(c.name).to_str().unwrap(), "Acid Bass");
            assert_eq!(CStr::from_ptr(s.id).to_str().unwrap(), "com.ice1x.seclab");
            // Vendor, version and features come from the plugin, unchanged.
            assert_eq!(CStr::from_ptr(c.vendor), CStr::from_ptr(s.vendor));
            assert_eq!(CStr::from_ptr(c.version), CStr::from_ptr(s.version));
            assert_eq!(
                CStr::from_ptr(*c.features).to_str().unwrap(),
                CStr::from_ptr(*s.features).to_str().unwrap()
            );
        }
    }

    #[test]
    fn the_factory_exposes_exactly_one_plugin_and_refuses_foreign_ids() {
        let null_factory = std::ptr::null();
        // SAFETY: the factory functions ignore the factory pointer; ids are valid C strings.
        unsafe {
            assert_eq!(get_plugin_count(null_factory), 1);
            // A test binary is not a bundle, so the built-in identity is reported.
            let d = get_plugin_descriptor(null_factory, 0);
            assert_eq!(
                CStr::from_ptr((*d).id).to_str().unwrap(),
                "com.ice1x.seclab"
            );
            assert!(get_plugin_descriptor(null_factory, 1).is_null());
            // Unknown or null ids and a null host never create an instance.
            let host = std::ptr::NonNull::dangling().as_ptr();
            assert!(create_plugin(null_factory, host, c"com.other".as_ptr()).is_null());
            assert!(create_plugin(null_factory, host, std::ptr::null()).is_null());
            assert!(
                create_plugin(null_factory, std::ptr::null(), c"com.ice1x.seclab".as_ptr())
                    .is_null()
            );
        }
    }
}
