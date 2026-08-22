//! `vb-editor` — egui node editor (canvas, wires, inspector, undo).
//!
//! Epic 4. This crate owns the visual editor: an infinite canvas viewed through a
//! [`Camera`], with node rendering, wire drag-and-drop, the palette, and undo/redo
//! layered on top task by task. The standalone and (later) the plugin embed
//! [`EditorApp`]; `cargo run -p vb-editor` opens it as its own window.

pub mod app;
pub mod camera;
pub mod layout;
pub mod module;
pub mod palette;
pub mod patch;
pub mod preset;
pub mod theme;
#[cfg(all(target_arch = "wasm32", feature = "app"))]
pub mod web;
pub mod widgets;

#[cfg(all(not(target_arch = "wasm32"), feature = "app"))]
pub use app::run;
pub use app::EditorApp;
pub use camera::{Camera, MAX_ZOOM, MIN_ZOOM};
pub use layout::{hit_test, layout_node, NodeHit, NodeLayout, NodeStyle};
pub use module::{list_modules, MacroModule, ModuleError};
pub use palette::{palette_entries, PaletteEntry};
pub use patch::PatchView;
pub use preset::{Preset, PresetBank, PresetError, PresetValue, PRESET_VERSION};
pub use theme::{Theme, ThemeKind};
pub use widgets::WidgetKind;
