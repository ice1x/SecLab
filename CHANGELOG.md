# Changelog

All notable changes to VST_bomb are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Versioning

Two independent version numbers matter in this project:

- **Crate / release version** — a single workspace-wide version (`version.workspace` in the
  root `Cargo.toml`), currently **0.0.0**. The project is **pre-1.0**: it has not cut a
  release yet, so everything to date lives under _Unreleased_ below. Once releases begin,
  this version follows [Semantic Versioning](https://semver.org/) — MAJOR.MINOR.PATCH — and
  each release moves the accumulated _Unreleased_ notes under a dated heading.
- **Project format version** — the on-disk `.ron` patch format has its own version
  (`vb_project::FORMAT_VERSION`), currently **1**, with backward loading supported down to
  `MIN_SUPPORTED_VERSION` via the migration path. This number changes only when the patch
  file schema changes, independently of the crate version. Bumping it belongs in a
  _Changed_ entry here.

## [Unreleased]

The first development cycle, building the engine, node library, editor, plugin export, and
the AI assistant from the ground up. Highlights by area:

### Added

- **Graph engine (`vb-core`)** — a validated node/port/edge graph and an allocation-free,
  topologically-scheduled block processor; signal-typed ports (audio / control) with
  connection rules (type match, single-driver inputs, no cycles).
- **Node library (`vb-nodes`)** — oscillator (anti-aliased), noise, biquad and
  state-variable filters, ADSR envelope, tempo-syncable LFO, math/routing nodes, delay and
  reverb, waveshaper, MIDI input, polyphony/voice allocation, audio I/O, and macro-nodes
  (nested subgraphs). Control-rate modulation via **VCA** (amplitude), **VCF** (cutoff),
  and **VCO** (pitch). A public registry for third-party nodes and an auto-generated node
  reference.
- **Project format (`vb-project`)** — RON `ProjectDoc` (format version 1) with structural
  validation, a migration path, parameter mapping/automation, and a modulation matrix.
- **Runtime (`vb-runtime`)** — real-time engine with lock-free background rebuilds for
  editing a running patch without dropouts.
- **Editor (`vb-editor`)** — egui node canvas (pan/zoom), node rendering, drag-to-wire,
  parameter inspector, undo/redo with gesture coalescing, macro modules, presets/banks,
  themes, and applying an assistant proposal as one undoable, auditioned command.
- **Standalone & plugin (`vb-standalone`, `vb-plugin`)** — a live standalone editor with a
  cpal audio stream, and a CLAP instrument (nih-plug) that compiles a patch into the
  runtime engine and streams it to the host; VST3/AU via clap-wrapper, validated in CI with
  clap-validator.
- **Signing & notarization (`xtask`)** — `cargo xtask sign` / `cargo xtask notarize`
  code-sign the macOS bundles with a Developer ID identity (hardened runtime + secure
  timestamp) and notarize + staple them via `notarytool`, driven by `$DEVELOPER_ID` /
  `$NOTARY_PROFILE`. The order-sensitive tool argument vectors are pure, unit-tested
  functions; the full process is written up in [`docs/signing.md`](docs/signing.md).
- **AI assistant (`vb-ai`)** — a grounded, offline-testable assistant stack: system prompt
  built from the live node reference and the RON output contract; a provider abstraction
  with a BYOK keystore and a privacy gate (offline switch, consent, a "what will be sent?"
  payload inspector); a token/cost budget guard; propose → validate → repair convergence;
  and generate / edit / explain / suggest / repair / critique skills.
- **Anthropic provider adapter (`vb-ai`)** — the reference `AiProvider` speaking the
  Anthropic Messages API. The request/response codec is a pure, always-compiled,
  unit-tested pair over an injectable `HttpTransport` seam; the real blocking HTTP
  transport (`ReqwestTransport`, rustls) lives behind the optional `net` feature, so the
  default build (and CI) links no HTTP stack and can send nothing without an explicit
  opt-in and a BYOK key.
- **OpenAI-compatible provider adapter (`vb-ai`)** — one `AiProvider` for the Chat
  Completions family (OpenAI and the many gateways/local servers that copy its shape),
  with a configurable endpoint. Same offline-tested codec + injectable-transport structure
  as the Anthropic adapter; the system prompt maps to the leading `system` message and the
  reply text to `choices[0].message.content`. Shares the `net`-gated HTTP transport.
- **Offline / local models (`vb-ai`)** — a `LocalProvider` reaching a local LLM server
  (Ollama, LM Studio, llama.cpp) through the same `AiProvider` trait and OpenAI-compatible
  codec, pointed at a `localhost` endpoint and keyless by default (an optional bearer for a
  secured local gateway). The request never leaves the machine — no BYOK key, no cloud
  round-trip — so it is the privacy-preserving assistant path.
- **Patch library (`vb-library`, optional)** — behind the opt-in `library` feature, an
  integration with the embedded [drevo](https://github.com/) graph store via its direct
  Rust API (`redb-backend`, no Bolt server), assessed with a CRUD smoke test. Off by
  default, so the standard build never compiles it.
- **WASM proof of concept (`vb-editor`)** — the editor compiles to
  `wasm32-unknown-unknown` and runs in a browser through eframe's `WebRunner` (WebGL) via a
  cfg-split entry point (`vb_editor::web::start`), leaving the native winit launcher
  untouched; the platform-agnostic core (`vb-core` / `vb-nodes` / `vb-project`) is portable
  too, so `.vbp` files load on both. A non-blocking CI `wasm build` job compiles them on
  every push. Build/serve steps in [`docs/wasm.md`](docs/wasm.md). (Browser audio is a
  separate, later task.)
- **In-app project serialization (`vb-editor`)** — `EditorApp::export_project_ron` /
  `import_project_ron` round-trip the whole patch through the `.vbp` RON format using only
  `vb-project` and in-memory strings (no filesystem), wired to an egui text-buffer panel.
  Because it avoids browser file APIs, graph editing **and** save/load work in the WASM
  build, closing the browser-editing loop; a failed import leaves the canvas untouched.
- **Web/native runtime boundary** — the shared interchange point is the `.vbp` project
  format (`vb-project`); the engine (`vb-core`/`vb-runtime`), nodes, editor, and assistant
  core all build to `wasm32`, while audio device I/O (`cpal`), plugin export (`nih-plug`),
  and the drevo knowledge graph are native-only. The `library` feature does **not** build
  to wasm — `getrandom` (via `uuid`→`drevo`) and `redb`'s mmap store — verified and guarded
  by the CI `wasm build` job (which now also builds `vb-runtime` and the default
  `vb-library`). Written up in
  [`docs/architecture-boundary.md`](docs/architecture-boundary.md).
- **Patches carry per-node parameter values (`vb-project`, `vb-plugin`)** — a `.vbp`
  document is now structure **and** knob settings, not just a graph. `NodeEntry` grew an
  optional `values` list (registry `params()` order) that round-trips through RON and is
  omitted when empty, so structure-only and pre-existing documents load unchanged. The
  plugin's engine builder honours a node's stored values when nothing overrides them, so a
  preset can pin a specific sound; host automation still overrides per node at rebuild. This
  is the enabler for the factory preset library (Epic 13) and a silent-until-note default.
  Threading values correctly also fixed a latent id bug: the builder now addresses runtime
  nodes through `ProjectDoc::to_graph_with_remap` instead of assuming document and runtime
  ids coincide.
- **Silent-until-note default patch (`vb-plugin`)** — the shipped patch is now a
  note-gated voice — `oscillator → svf → vca.in` for audio, `midi-in.gate → adsr → vca.cv`
  for control — with the VCA stored biased shut (`bias = 0`, so `out = in · cv`). The
  envelope is the only thing that opens it, so the instrument no longer drones the instant
  it loads in a DAW; it is silent at rest and sounds when a note plays. Loading a preset
  also seeds the host/editor parameter defaults from its stored values (via
  `vb_project::build_mappings`), so the VCA's `bias = 0` actually takes effect instead of
  reverting to the registry default.
- **Ladder-filter node (`vb-nodes`, kind `ladder`)** — a Moog-style four-pole
  (24 dB/octave) resonant low-pass, control-swept like the `vcf`
  (`cutoff = clamp(base_hz + amount_hz · cv)`). Built from `tan`-prewarped TPT one-pole
  stages closed with a **zero-delay-feedback** resonance loop (Zavalishin), so the cutoff
  tunes accurately and it stays stable right up to self-oscillation instead of blowing up
  the way a naive delayed-feedback ladder does. Steeper roll-off and singing resonance the
  two-pole `svf`/`biquad` can't give — the shared filter prerequisite for the Epic 13 Moog
  clones (000143). Ships the env-swept `examples/ladder_bass.ron` factory patch (silent at
  rest, sings on a note) and is in the generated [node reference](docs/nodes.md).
- **Pitch-tracking oscillator node (`vb-nodes`, kind `pitch-osc`)** — the missing link
  between `midi-in` and audible pitch: it reads the played note's absolute frequency
  (`midi-in.pitch_hz`, in Hz) from a control input and retunes its oscillator to it each
  block, falling back to `base_hz` when no note is held. Plain `oscillator` runs at a fixed
  frequency and `vco` takes an *exponential* octave CV — neither tracks absolute Hz — so
  this is what makes a keyboard-tracking (playable) voice possible, the enabler for the
  Epic 13 clones (000144). Ships the playable `examples/mono_lead.ron` (midi → `pitch-osc`
  → `ladder` → env-gated `vca`), whose test asserts a higher note sounds higher through the
  whole engine; regenerated [node reference](docs/nodes.md).
- **Minimoog Model D factory clone (`examples/minimoog.ron`, Epic 13 / 000137)** — the
  first full playable clone: three detuned pitch-tracking oscillators (unison, +6 cents,
  and one an octave down) mixed into an envelope-swept `ladder` and a note-gated VCA. It
  tracks the played pitch, stays silent at rest, and sounds on a note (all asserted in the
  examples test). `pitch-osc` gained a `detune_semitones` parameter (±24, backward
  compatible) to build the oscillator stack that gives the voice its fatness.
- **Glide (portamento) node (`vb-nodes`, kind `glide`)** — a control-rate slew that ramps
  its input toward each new value over `glide_ms`, so a pitch wired through it slides
  between notes instead of jumping. Wraps the existing `Smoother`; the first value snaps in
  (a patch sounds at pitch on the first note) and `glide_ms = 0` is transparent pass-through.
- **Moog Minitaur factory clone (`examples/minitaur.ron`, Epic 13 / 000136)** — a mono bass
  voice: two detuned saws plus a square sub an octave down (`pitch-osc`), a `glide` stage on
  the pitch, an envelope-swept `ladder`, and a note-gated VCA. Silent at rest, plays on a
  note, tracks pitch, and carries portamento (asserted in the examples test).
- **Behringer Neutron factory clone (`examples/neutron.ron`, Epic 13 / 000139)** — two
  detuned pitch-tracking oscillators through an envelope-swept multimode `vcf`, a
  `waveshaper` overdrive and a note-gated VCA, with an in-voice `fb-delay` mixed back in
  (dry + wet) so the echoes ring on past the gate. Built entirely from existing nodes;
  playable, silent at rest, sounds on a note (asserted in the examples test).
- **Moog Grandmother factory clone (`examples/grandmother.ron`, Epic 13 / 000138)** — a
  saw + triangle pair (`pitch-osc`) through an envelope-swept `ladder` and a note-gated VCA
  into the `reverb` node (Freeverb, generous dry/wet) for the semi-modular's spring-reverb
  ambience. Built from existing nodes; playable, silent at rest, sounds on a note.
- **Documentation** — a [node reference](docs/nodes.md), a
  [user guide](docs/first-plugin.md) (build your first plugin), a
  [developer guide](docs/adding-a-node.md) (add your own node), a
  [real-host testing checklist](docs/host-testing.md) (manual DAW QA — kept in sync with the
  produced bundles by an xtask test), a [signing/notarization guide](docs/signing.md), and
  tested starter patches under [`examples/`](examples).

### Fixed

- **AU (AudioUnit) bundle now registers and is branded (`xtask bundle-auv2`)** — clap-wrapper
  generates a correct `AudioComponents` plist but installed it into the `.component` only
  flakily, so some builds produced a bundle with an `APPL` plist and no registration
  (`auval` "didn't find the component"). `bundle-auv2` now installs the generated plist
  deterministically and rebrands the manufacturer, so the AU validates (`auval` succeeds)
  and hosts list it as **`ice1x: VST_bomb`** instead of clap-wrapper's default
  `cleveraudio.org: vb-plugin`.

### Changed

- **The full node editor now runs inside the plugin window (`vb-plugin`)** — the read-only
  run-view (000067) is replaced by the real `vb_editor::EditorApp`, rendered through
  `nih_plug_egui` into the plugin's own window: the same editor as the standalone, seeded
  with the patch the instance is playing. A structural edit mirrors the new graph into the
  shared project doc and kicks the off-audio-thread engine rebuild, so **the sound follows
  the canvas inside a DAW**. `EditorApp` grew a frame-independent `render(ctx)` (called by
  both eframe and the plugin host) and `load_doc`; `eframe` is now optional behind
  `vb-editor`'s `app` feature, so the plugin embeds the editor **without** pulling
  winit/eframe. Inherent plugin-format limits: the host-automatable parameter list is fixed
  at load (parameters of nodes added in-session appear to host automation only after a
  reload), and the rebuild is structural (new nodes take registry-default parameter values).
  The `gui` build and its edit→rebuild wiring compile-check in CI; the interactive/audible
  result is verified in a real host.
- **Unified egui on 0.31** — the workspace editor stack (`vb-editor`, `vb-standalone`) moved
  from egui/eframe 0.36 to **0.31**, the version the plugin's in-window egui host
  (`nih_plug_egui`) already uses. The tree previously carried two egui versions side by
  side; now there is **one**. This removes the version wall that blocked reusing the real
  `EditorApp` inside the plugin window, so the same editor code can render natively, in the
  browser, and (next) embedded in a DAW — one shared codebase instead of a second,
  drifting implementation. No behavioural change; native/wasm builds, tests, clippy
  (`-D warnings`), and the `gui` build all stay green.

[Unreleased]: https://github.com/ice1x/VST_bomb/commits/main
