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

## Not covered here

- **Audio** — the browser build has no `cpal` stream; running the engine under
  WebAudio/AudioWorklet is [000083](../README.md).
- **Persistence** — loading/saving `.vbp` in the browser (file picker / local storage) is
  [000082](../README.md).
