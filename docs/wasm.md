# Running the editor in a browser (WASM)

The VST_bomb editor is platform-agnostic [egui](https://github.com/emilk/egui): the same
[`EditorApp`](../crates/vb-editor/src/app.rs) that opens a native window also runs in a
browser through eframe's `WebRunner`, which paints into an HTML `<canvas>` via WebGL. This
is a **proof of concept** (backlog 000081) — it establishes the web build path. Audio does
not run in the browser yet (that is [000083](../README.md), WebAudio/AudioWorklet); the web
build is the editor without sound.

## What is portable today

- **Core** — `vb-core` (engine/graph model), `vb-nodes` (node registry), and `vb-project`
  (the RON project format) all build for `wasm32-unknown-unknown`. The same `.vbp` files
  load on native and web, which is the shared-format foundation for
  [000084](../README.md).
- **Editor** — `vb-editor` builds for `wasm32` with a browser entry point,
  [`vb_editor::web::start`](../crates/vb-editor/src/web.rs). The native launcher
  (`vb_editor::run`, winit) and the web launcher are cfg-split, so neither pulls the
  other's dependencies.

The CI `wasm build` job compiles all of the above on every push (non-blocking).

## Build it

```bash
rustup target add wasm32-unknown-unknown

# Quick compile check (what CI runs):
cargo build -p vb-editor --target wasm32-unknown-unknown
```

To produce something a browser can load, run the JS-binding step with
[`wasm-bindgen`](https://github.com/rustwasm/wasm-bindgen) or, more simply,
[`trunk`](https://trunkrs.org):

```bash
cargo install wasm-bindgen-cli    # or: cargo install trunk

# with wasm-bindgen directly:
cargo build -p vb-editor --target wasm32-unknown-unknown --release
wasm-bindgen target/wasm32-unknown-unknown/release/vb_editor.wasm \
  --out-dir web/pkg --target web
```

Then a minimal page imports the module and hands it a canvas:

```html
<canvas id="vb"></canvas>
<script type="module">
  import init, { start } from './pkg/vb_editor.js';
  await init();
  await start(document.getElementById('vb'));
</script>
```

Serve the directory over HTTP (WebGL needs a real origin, not `file://`):

```bash
python3 -m http.server --directory web 8080
# open http://localhost:8080
```

## Editing and serialization in the browser

Graph editing works in the web build, and so does save/load: the palette panel has a
**project (RON)** section whose Export/Import buttons round-trip the whole patch through the
`.vbp` format (`EditorApp::export_project_ron` / `import_project_ron`). It uses only
`vb-project` and an in-memory text buffer — no browser file API — so copy the RON out to
save, or paste a `.vbp` in and load it. This is [000082](../README.md).

## Audio in the browser (WebAudio / AudioWorklet)

The instrument engine runs in the browser too — not through `cpal` (which the wasm build
lacks) but through a WebAudio `AudioWorklet`. The engine core lives in the nih-plug-free
[`vb-engine`](../crates/vb-engine) crate, compiled to `wasm32` and driven from JS via its
`WasmSynth` binding (`new` / `noteOn` / `noteOff` / `render`, one 128-sample quantum per
`process()` call). A runnable prototype — demo page, the worklet processor, and a headless
Node smoke test of the whole Rust → wasm → JS path — is in [`web/audio/`](../web/audio),
with build/run/test steps in its README. This is [000083](../README.md).

## Not covered here

- **Native file dialogs / local storage** — the in-app RON buffer is the portable path;
  richer browser persistence (a file picker, `localStorage`) can build on it later.
