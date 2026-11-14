//! Export a SecLab patch as its **own** plugin.
//!
//! - [`identity`] — the name + CLAP id a bundle presents (`identity.ron`), read by the plugin's
//!   CLAP entry point and written by the exporter (000265).
//! - [`bundle`] — clones the installed stock SecLab CLAP/VST3/AU into `<Name>.*` bundles that
//!   carry the patch and identity, re-signs and installs them (000266).
//!
//! Deliberately small and host-framework-free (no nih-plug, no egui) so both the plugin and the
//! editor can depend on it.

pub mod bundle;
pub mod identity;

pub use bundle::{export_plugin, ExportError, Exported, PluginDirs, Templates};
pub use identity::PluginIdentity;
