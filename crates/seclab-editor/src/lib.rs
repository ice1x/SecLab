//! `seclab-editor` — egui node editor (canvas, wires, inspector, undo).
//!
//! Epic 4. This crate owns the visual editor: an infinite canvas viewed through a
//! [`Camera`], with node rendering, wire drag-and-drop, the palette, and undo/redo
//! layered on top task by task. The standalone and (later) the plugin embed
//! [`EditorApp`]; `cargo run -p seclab-editor` opens it as its own window.

pub mod app;
// The assistant chat panel needs the `seclab-ai` provider stack, so it lives behind the
// off-by-default, native-only `assistant` feature (see Cargo.toml). Gating the module keeps
// the wasm and plugin builds free of the HTTP/provider dependencies.
#[cfg(feature = "assistant")]
pub mod assistant_panel;
pub mod camera;
pub mod factory;
pub mod layout;
pub mod library;
pub mod library_panel;
pub mod module;
pub mod palette;
pub mod patch;
pub mod preset;
#[cfg(feature = "assistant")]
pub mod report_panel;
#[cfg(feature = "assistant")]
pub mod settings_panel;
pub mod theme;
#[cfg(all(target_arch = "wasm32", feature = "app"))]
pub mod web;
pub mod widgets;

#[cfg(all(not(target_arch = "wasm32"), feature = "app"))]
pub use app::run;
pub use app::{EditorApp, PanelBinding, PanelMode, PanelModule, ViewMode};
#[cfg(feature = "assistant")]
pub use assistant_panel::{assistant_panel_ui, AssistantAction};
pub use camera::{Camera, MAX_ZOOM, MIN_ZOOM};
pub use factory::{factory_patch, FactoryPatch, FACTORY_PATCHES};
pub use layout::{hit_test, layout_node, NodeHit, NodeLayout, NodeStyle};
pub use library::{
    library_from_module_dir, library_items_from_module_dir, LibraryBrowser, LibraryItem,
};
pub use library_panel::library_browser_ui;
pub use module::{list_modules, user_modules_dir, MacroModule, ModuleError};
pub use palette::{palette_entries, PaletteEntry};
pub use patch::PatchView;
pub use preset::{Preset, PresetBank, PresetError, PresetValue, PRESET_VERSION};
#[cfg(feature = "assistant")]
pub use settings_panel::{settings_panel_ui, SettingsAction};
pub use theme::{Theme, ThemeKind};
pub use widgets::WidgetKind;
