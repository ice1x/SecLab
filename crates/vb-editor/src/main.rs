//! Launches the VST_bomb node editor window.
//!
//! Only meaningful for the native standalone build (the `app` feature). On `wasm32` the
//! editor is booted from JavaScript through [`vb_editor::web::start`], and when `app` is
//! off (the editor embedded in another host, e.g. the plugin) this binary is a no-op.

#[cfg(all(not(target_arch = "wasm32"), feature = "app"))]
fn main() -> Result<(), eframe::Error> {
    vb_editor::run()
}

#[cfg(not(all(not(target_arch = "wasm32"), feature = "app")))]
fn main() {}
