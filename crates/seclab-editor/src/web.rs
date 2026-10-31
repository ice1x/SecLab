//! The browser entry point (backlog 000081 — WASM proof of concept).
//!
//! The whole editor — canvas, camera, node rendering, wiring, inspector, undo — is
//! platform-agnostic egui, so the same [`EditorApp`](crate::EditorApp) that runs natively
//! also runs in a browser: only the launcher differs. Native uses
//! [`run`](crate::app::run) (winit + an OS window); the web uses eframe's `WebRunner`,
//! which paints into an HTML `<canvas>` via WebGL. This module is compiled only for
//! `wasm32`, so it adds nothing to the native build.
//!
//! A page bundles this with `wasm-bindgen` (or `trunk`) and calls [`start`] with its
//! canvas element. See [`docs/wasm.md`](../../docs/wasm.md) for the build/serve steps.

use wasm_bindgen::prelude::*;

use crate::EditorApp;

/// Boots the editor into the given `<canvas>` element and runs it.
///
/// Exposed to JavaScript as `start(canvas)`; returns once the app is initialised (the
/// render loop continues on the browser's animation-frame callbacks).
///
/// # Errors
/// A [`JsValue`] if eframe cannot initialise the WebGL context or create the app.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    // Log panics to the browser console instead of an opaque "unreachable executed".
    console_error_panic_hook::set_once();

    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|_cc| Ok(Box::new(EditorApp::new()))),
        )
        .await
}
