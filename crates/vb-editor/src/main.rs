//! Launches the VST_bomb node editor window.
//!
//! Native only: on `wasm32` the editor is booted from JavaScript through
//! [`vb_editor::web::start`], not from a `main`, so this binary compiles to a no-op there.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), eframe::Error> {
    vb_editor::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
