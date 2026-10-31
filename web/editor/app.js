// Boots the VST_bomb editor (Rust → WASM, eframe's WebRunner) into the page's <canvas>.
//
// The whole editor — canvas, palette (incl. the library section), inspector, undo — is
// platform-agnostic egui, so this is the same `EditorApp` that runs natively; only the
// launcher differs. `pkg/seclab_editor.js` is produced by wasm-pack `--target web`
// (see README.md); it exports the async `start(canvas)` from crates/seclab-editor/src/web.rs
// plus a default init that fetches and instantiates the wasm module.
import init, { start } from './pkg/seclab_editor.js';

const loading = document.getElementById('loading');
const canvas = document.getElementById('vb-canvas');

// A self-test hook the in-app browser (and CI, later) can read without a human looking:
// `window.__vbEditorReady` flips to true once the editor is running, or to an error string.
window.__vbEditorReady = false;

async function main() {
  try {
    await init(); // fetch + instantiate vb_editor_bg.wasm
    await start(canvas); // hand the canvas to eframe's WebRunner; the render loop then runs
    loading.remove(); // the editor is painting — drop the overlay
    window.__vbEditorReady = true;
  } catch (err) {
    loading.classList.add('error');
    loading.innerHTML = 'Failed to start the editor.<br><small>' + String(err) + '</small>';
    window.__vbEditorReady = 'error: ' + String(err);
    throw err;
  }
}

main();
