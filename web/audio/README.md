# Browser audio prototype (backlog 000083)

The SecLab instrument engine (Rust → WASM) running in the browser through a WebAudio
`AudioWorklet`. The DSP core is the `seclab-engine` crate, compiled to `wasm32` and driven from
JavaScript via its `WasmSynth` binding (`new` / `noteOn` / `noteOff` / `render` — one
128-sample quantum per `AudioWorklet` `process()` call).

## Files

| File | Runs in | Role |
| --- | --- | --- |
| `index.html` | main thread | demo page: Start button, self-test, QWERTY keyboard |
| `app.js` | main thread | loads the wasm, assembles the worklet module, maps keys → MIDI |
| `seclab-processor.js` | audio thread | the `AudioWorkletProcessor`: owns the synth, renders each quantum |
| `text-polyfill.js` | audio thread | minimal `TextEncoder`/`TextDecoder` (absent from `AudioWorkletGlobalScope`) |
| `test/smoke.mjs` | Node | headless end-to-end test of the wasm audio boundary |

The worklet module is assembled at runtime by concatenating `text-polyfill.js` + the
wasm-bindgen `no-modules` glue + `seclab-processor.js` into one Blob, because
`AudioWorkletGlobalScope` supports neither ES `import` nor `importScripts`. The wasm bytes
are handed to the processor via `processorOptions` and instantiated with
`wasm_bindgen.initSync`.

## Run it in a browser

Build the `no-modules` package the worklet loads, then serve this directory:

```bash
# from the repo root
wasm-pack build crates/seclab-engine --target no-modules --out-dir ../../web/audio/pkg-nomod --out-name seclab_engine
python3 -m http.server --directory web/audio 8099
# open http://localhost:8099 — click "Start audio", then play with a s d f … / w e t y u
```

`AudioContext` needs a user gesture, so audio starts on the **Start** button. The
**Self-test** button plays A4 for a moment and reports the measured output peak
(`peak=… PASS`), so the whole path can be confirmed without listening.

## Headless test (what CI runs)

Build the Node-target package and run the smoke test — no browser required:

```bash
# from the repo root
wasm-pack build crates/seclab-engine --target nodejs --out-dir ../../web/audio/pkg-node --out-name seclab_engine
node web/audio/test/smoke.mjs
```

It asserts the synth is silent at rest, audible on a note, decays after release, renders
128-sample quanta, and reports a malformed patch as a thrown error.

> The `pkg-*/` output directories are generated (git-ignored); only the source is committed.
