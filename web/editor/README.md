# SecLab editor in the browser (WASM)

A runnable web host for the editor: the same [`EditorApp`](../../crates/seclab-editor/src/app.rs)
that opens a native window, painting into an HTML `<canvas>` through eframe's `WebRunner`
(WebGL). This is the browser front-end for the WASM proof of concept
([000081](../../docs/wasm.md)) — graph editing, the palette (including the **library**
section), the inspector, undo/redo, and RON project import/export all work. Audio does not
run here (that is the separate WebAudio path in [`web/audio/`](../audio)); this is the editor
without sound.

## Build

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack   # once

# From the repo root: compile seclab-editor to a browser-loadable wasm package.
wasm-pack build crates/seclab-editor --target web \
  --out-dir ../../web/editor/pkg --out-name seclab_editor
```

`--out-dir` is relative to the crate (`crates/seclab-editor`), so `../../web/editor/pkg`
lands the package here. The compiled `pkg/` is git-ignored (like `web/audio/pkg*`); the
source (`index.html`, `app.js`) is committed. Add `--dev` for a faster, unoptimized build.

## Serve

WebGL needs a real HTTP origin (not `file://`):

```bash
python3 -m http.server --directory web/editor 8080
# open http://localhost:8080
```

## How it loads

`app.js` imports the wasm-pack module, `await init()`s it (fetch + instantiate
`vb_editor_bg.wasm`), then calls the exported `start(canvas)` from
[`crates/seclab-editor/src/web.rs`](../../crates/seclab-editor/src/web.rs), which hands the canvas to
eframe's `WebRunner`. Once the editor is painting, `app.js` sets `window.__vbEditorReady =
true` (or an error string) — a headless readiness hook a browser/CI check can read without a
human looking at the page.

## Save / load in the browser

The palette's **project (RON)** section round-trips the whole patch through the `.vbp`
format with an in-memory text buffer (no browser file API): copy the exported RON out to
save, or paste a `.vbp` in and load it. See [000082](../../docs/wasm.md).
