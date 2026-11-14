//! The plugin side of the runtime identity (000265): our own CLAP entry point.
//!
//! The identity itself (`identity.ron`: name + CLAP id, validation, where a bundle keeps it)
//! lives in `seclab-export`, shared with the exporter that writes it. This module reads it
//! for the bundle this dylib was loaded from and reports it to the host.

pub use seclab_export::identity::{
    identity_for_dylib, resources_identity_path, IdentityError, PluginIdentity, IDENTITY_FILE,
};

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
    use nih_plug::prelude::{ClapPlugin, Plugin};
    use std::ffi::CStr;

    fn acid() -> PluginIdentity {
        PluginIdentity {
            name: "Acid Bass".into(),
            clap_id: "com.ice1x.seclab.user.acid-bass".into(),
        }
    }

    #[test]
    fn the_builtin_identity_is_the_compiled_plugin_identity() {
        assert_eq!(crate::SecLab::NAME, PluginIdentity::builtin().name);
        assert_eq!(crate::SecLab::CLAP_ID, PluginIdentity::builtin().clap_id);
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
