# The web / native runtime boundary

SecLab runs in three places — a native desktop editor, an audio plugin inside a DAW, and
(as of Epic 9) a browser — from **one** codebase. This document defines what is shared
across those targets and what is target-specific, so a change lands on the right side of the
line. It is the design deliverable for [000084](../README.md); the WASM verification for
[000099](../README.md) is folded in below.

## The one shared thing: the project format

The **`.vbp` project** (`vb-project`, RON, `FORMAT_VERSION`) is the contract every target
shares. A patch authored in the desktop editor loads in the plugin and in the browser
because all three parse the same document into the same `vb-core` graph. Everything else —
how a patch is *edited*, *played*, or *stored* — is layered on top and may differ per
target. Keep the format the stable interchange point; do not smuggle target-specific state
into it (canvas positions, audio-device settings, and keys all live outside `.vbp`).

## What builds where (verified)

Measured with `cargo build -p <crate> --target wasm32-unknown-unknown` on the current tree:

| Crate | Native | wasm32 | Role |
| --- | :---: | :---: | --- |
| `vb-core` | ✅ | ✅ | graph model + block-scheduled engine |
| `vb-nodes` | ✅ | ✅ | node registry + DSP |
| `vb-project` | ✅ | ✅ | **the shared `.vbp` format** |
| `vb-runtime` | ✅ | ✅ | RT executor (buffer processing) |
| `vb-editor` | ✅ | ✅ | egui editor (native winit / web `WebRunner`) |
| `vb-ai` (default) | ✅ | ✅ | assistant grounding + offline logic |
| `vb-library` (default) | ✅ | ✅ | empty façade without the `library` feature |
| `vb-ai` (`net`) | ✅ | — | `reqwest` HTTP transport is native |
| `vb-library` (`library`) | ✅ | ❌ | drevo/redb knowledge graph — see below |
| `vb-standalone` | ✅ | — | `cpal` audio-device host |
| `vb-plugin` | ✅ | — | CLAP/`nih-plug` — a host-loaded native ABI |
| `xtask` | ✅ | — | build/sign tooling |

The key insight: **audio *execution* is portable, audio *I/O* is not.** `vb-core` /
`vb-runtime` process buffers on wasm just fine; what is native-only is the device stream
(`cpal` in `vb-standalone`) and the plugin ABI (`nih-plug` in `vb-plugin`). That is exactly
why the browser audio task ([000083](../README.md)) is "engine in WASM + WebAudio/AudioWorklet
for the stream", not a rewrite of the engine.

## Native-only, and why

- **Audio device I/O** — `cpal` (`vb-standalone`) talks to CoreAudio/ALSA/WASAPI; the browser
  equivalent is WebAudio, a different API reached from JS. The engine feeding it is shared.
- **Plugin export** — `vb-plugin` compiles to a CLAP/VST3/AU binary loaded by a DAW over a
  native ABI; there is no browser analogue.
- **The drevo knowledge graph** — `vb-library --features library` does **not** build to
  wasm32 (000099, verified). The first hard error is `getrandom v0.4.3` (pulled by
  `uuid` → `drevo`) refusing wasm without its `wasm_js` feature; beyond that, drevo's `redb`
  backend is a memory-mapped file store with no browser-sandbox equivalent. The library is
  therefore a **native-only** capability. This is safe by construction: the `library`
  feature is off by default, so the default `vb-library` builds to wasm (it is an empty
  façade), and the shared web path uses the **format** (`vb-project`), never the library.
- **BYOK network** — `vb-ai --features net` uses `reqwest`; the browser would use `fetch`
  instead. The assistant's grounding and offline logic (default `vb-ai`) is shared.
- **Build tooling** — `xtask` (bundling, signing) is a developer-machine concern.

## How the boundary is enforced

Structurally, not by discipline:

- **Cargo features** gate the native-only heavy deps (`library`, `net`, `gui`) off by
  default, so the portable crates never pull them.
- **`cfg(target_arch)`** splits the per-target glue in place — e.g. `vb-editor`'s native
  `run()` vs. the wasm `web::start()` (see [`docs/wasm.md`](wasm.md)).
- **CI** builds the portable set (`vb-core`, `vb-nodes`, `vb-project`, `vb-editor`, and the
  default `vb-library`) for `wasm32-unknown-unknown` on every push (the non-blocking
  `wasm build` job), so a change that accidentally makes a shared crate native-only is
  caught.

## Rule of thumb

New shared logic (a node, a format change, engine work, assistant grounding) → a portable
crate, no native-only dep. New device/host/tooling integration → behind a feature or a
`cfg(target_arch)` split, never in the shared path. When unsure, try
`cargo build -p <crate> --target wasm32-unknown-unknown`.
