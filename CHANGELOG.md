# Changelog

All notable changes to VST_bomb are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Versioning

Two independent version numbers matter in this project:

- **Crate / release version** — a single workspace-wide version (`version.workspace` in the
  root `Cargo.toml`), currently **0.0.1** — the first cut release. The project is **pre-1.0**,
  so the API and patch format may still change between releases. This version follows
  [Semantic Versioning](https://semver.org/) — MAJOR.MINOR.PATCH — and each release moves the
  accumulated _Unreleased_ notes under a dated heading.
- **Project format version** — the on-disk `.ron` patch format has its own version
  (`vb_project::FORMAT_VERSION`), currently **1**, with backward loading supported down to
  `MIN_SUPPORTED_VERSION` via the migration path. This number changes only when the patch
  file schema changes, independently of the crate version. Bumping it belongs in a
  _Changed_ entry here.

## [Unreleased]

### Changed

- **Editor (`vb-editor`)** — control precision. Time (`_s`, `_ms`) and frequency (`_hz`) sliders
  and faders now read on a **logarithmic** scale (`widgets::param_is_log`), so the musically useful
  low end has real travel: on a linear 0–5 s decay fader, 0.03 s and 0.15 s were a couple of pixels
  apart and unreachable; on the log scale a small drag moves cleanly between them (verified in the
  browser/WASM build). Levels (`sustain`, `resonance`), gains (`_db`) and bipolar ranges
  (`detune_semitones`) stay linear. Knobs got a longer full-sweep travel (`KNOB_RANGE_PX` 200 → 320)
  for finer control, and holding **Shift** while turning a knob trims at a quarter speed
  (`KNOB_FINE_FACTOR`).

### Added

- **Editor (`vb-editor`)** — the **Panel** view is now a rack of titled modules instead of a
  flat list of sliders. `EditorApp::panel_modules` groups the controls by node (one framed block
  per param-bearing node), numbering repeated kinds (`pitch-osc 1/2/3`) while leaving once-only
  kinds bare, and each module renders its parameters as rotary knobs live-bound through
  `set_param`. First step of Epic 15's interface constructor.
- **Editor (`vb-editor`)** — panel controls now honor each parameter's **widget type** (knob /
  slider / switch), and **right-clicking a control** switches its type in place. `PanelBinding`
  carries the `widget`, so the panel and the inspector stay in lock-step.
- **Editor (`vb-editor`)** — panel modules are **draggable**: grab a module's title bar to place
  it anywhere; unplaced modules flow in a wrapping layout. Placement is view-only
  (`PatchView::set_panel_position`) and does not rebuild the engine. Un-placed modules lay out in a
  real wrapping flow (egui measures each block's height, so tall modules never overlap the row
  below); dragging a module's title lifts it out of the flow into a free-positioned layer.
- **Project (`vb-project`) / Editor (`vb-editor`)** — the panel layout is **saved with the
  project**: `NodeEntry` gained optional `panel_pos` and `widgets` fields (both omitted when
  unset, so old documents still load and the format stays version 1), and the editor writes them
  in `to_doc` / restores them in `apply_doc`. A saved project reopens with its dragged module
  positions and chosen control types intact.
- **Plugin (`vb-plugin`)** — in-window edits are now saved into the **DAW project state**. The
  plugin re-serializes the live document into its host state on every edit (not just at load), so
  a rearranged/retyped panel — and any other in-window change — survives a project save/reopen.
  View-only edits (panel placement, control type) bump a new `PatchView::view_revision` and
  re-persist without rebuilding the audio engine.
- **Editor (`vb-editor`) / Project (`vb-project`)** — panel modules can be given an **accent
  colour** (right-click a module's title). The accent frames the module and tints its title, and
  is saved with the patch via a new optional `NodeEntry.panel_color` (`#rrggbb`) field.
- **Editor (`vb-editor`)** — a new **vertical fader** control type (`WidgetKind::Fader`), offered
  in the right-click menu and rendered by the panel and the inspector. Envelope segments (`*_s`
  times, `sustain`) now default to a fader, so an ADSR reads as a fader stack rather than a row of
  dials. The choice persists with the patch.

- **Plugin (`vb-plugin`)** — the plugin window is **resizable** again: a drag corner
  (`ResizableWindow`) lets you size the editor, using nih-plug's implemented `request_resize`
  path. The editor now draws inside the window's `ui` (`EditorApp::render_inside`, `show_inside`
  panels), fixing the layout clash that blacked out the earlier attempt. Host-frame-drag resize
  stays blocked (nih-plug's `ext_gui_set_size` is still a TODO); this is the plugin-initiated
  path other nih-plug plugins use.

### Fixed

- **Plugin window (`vb-plugin`)** — reverted the drag-corner resize (000191). On the AU surface
  the `ResizableWindow` path left the plugin **unclickable** after the drag was released: the host
  grew the window frame but the clap-wrapper→baseview egui surface did not follow, so input
  hit-testing desynced and the window went dead. That resize path is outside our layout and has no
  headless test to catch the regression, so we no longer ship it. The window returns to its working
  fixed size; the harmless `render_inside` refactor stays (standalone/browser render through it).
- **Editor (`vb-editor`)** — the Panel view no longer hides most modules. `horizontal_wrapped`
  does not wrap framed blocks, so the modules ran off the right edge in a single row and vanished;
  the panel now computes the column count from the available width (`panel_column_count`) and lays
  un-placed modules out row by row, so every module is visible and wraps into a grid. Verified in
  the browser (WASM) build.
- **Engine / Plugin editor (`vb-plugin`)** — turning a knob **in the plugin editor** during
  playback no longer clicks. The live-update path landed first, but the embedded editor still
  kicked a full engine rebuild on every audible edit — including a plain knob move — and that
  rebuild swapped in a fresh, state-reset engine, clicking anyway. The editor now rebuilds **only
  when the graph structure changes** (a node or wire added/removed); a parameter tweak is applied
  live through the host parameters.
- **Engine (`vb-runtime` / `vb-nodes` / `vb-plugin`)** — turning a knob during playback no longer
  corrupts the sound. A parameter change (knob or automation) used to rebuild the whole engine and
  swap it in, resetting every processor's DSP state (oscillator phase, filter memory, envelope
  stage) — a click on every change, ~60 a second while dragging. Parameters now update the running
  engine **in place** via a new `Processor::set_params` (implemented for every node in the default
  voice), so the sound retunes smoothly; a full rebuild happens only for structural edits or a node
  without live support.
- **Editor (`vb-editor`)** — exporting or saving a project no longer drops edited parameter
  values. `PatchView::to_doc` built the document from the graph shape alone, so every knob
  edited in the inspector reverted to its registry default on export/import; it now copies the
  inspector values into the document. Guarded by new end-to-end interface tests
  (`vb-editor/tests/e2e_interface.rs`) that drive `EditorApp` through the public UI API.

## [0.0.1] - 2026-09-14

The first cut release. The first development cycle, building the engine, node library,
editor, plugin export, and the AI assistant from the ground up. Highlights by area:

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
  every push. Build/serve steps in [`docs/wasm.md`](docs/wasm.md). (Browser **audio** is the
  `vb-engine` crate, below.)
- **Browser audio engine (`vb-engine`)** — the instrument core (`default_patch`,
  `engine_from_doc`) moved out of `vb-plugin` into a new crate that depends only on the DSP
  stack (`vb-core` / `vb-nodes` / `vb-project` / `vb-runtime`), so it carries **no** host
  framework and compiles to `wasm32-unknown-unknown`. `HeadlessSynth` drives a compiled patch
  block by block — build from the default patch or a `.vbp` RON string, feed MIDI note
  on/off, render fixed 128-sample quanta — and the wasm32-only `WasmSynth` (`wasm-bindgen`)
  wraps it as the object a WebAudio `AudioWorklet` calls (`new` / `fromRon` / `noteOn` /
  `noteOff` / `render`). `vb-plugin` re-exports the crate as `engine_builder`, so its API is
  unchanged; the CI `wasm build` job now also compiles `vb-engine`.
- **Browser audio prototype (`web/audio`, backlog 000083)** — the engine runs in the browser
  through a WebAudio `AudioWorklet`: a demo page loads the `vb-engine` wasm into the audio
  thread (the worklet module is the wasm-bindgen `no-modules` glue + a `TextEncoder`/
  `TextDecoder` polyfill — `AudioWorkletGlobalScope` has neither — + the processor, assembled
  into one Blob), a QWERTY keyboard sends MIDI note on/off, and each `process()` quantum is
  one `WasmSynth.render()`. A headless `node web/audio/test/smoke.mjs` verifies the whole
  Rust → wasm → JS path (silent at rest, audible on a note, decays on release, 128-sample
  blocks, malformed patch throws) and runs as a NON-blocking CI `browser audio` job. Verified
  end to end in a real browser via the page's self-test (plays A4, reports the measured output
  peak). Build/run/test steps in [`web/audio/README.md`](web/audio/README.md).
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
- **FM operator and supersaw oscillator nodes (`vb-nodes`, kinds `fm-op` / `supersaw`)** —
  two new pitch-tracking generators broadening the sound-source palette. `fm-op` is a sine
  carrier with a phase-modulation (`mod`) input and a harmonic `ratio`/`index`: chaining
  operators (`op → op.mod`) at musical ratios yields FM synthesis's bell, electric-piano and
  metallic timbres; with `index = 0` or nothing patched into `mod` it is a clean sine.
  `supersaw` is a unison stack of up to nine PolyBLEP sawtooths, symmetrically detuned around
  the tracked pitch and amplitude-normalised (`1/√N`), for fat trance/EDM leads. Both track
  `midi-in.pitch_hz` and rest on `base_hz`; unit-tested (pure sine at `ratio × pitch`, FM
  sidebands enrich the spectrum, detuned unison differs from a single saw, pitch tracking,
  bounded output) plus a registry test; regenerated [node reference](docs/nodes.md).
- **AD envelope and envelope-follower nodes (`vb-nodes`, kinds `ad` / `env-follower`)** —
  two additions filling out the thin envelope/modulation categories. `ad` is a one-shot
  attack→decay envelope: a rising gate edge fires the full contour and it ignores the gate
  release (a new edge re-triggers) — the "trigger and forget" shape percussion and plucks are
  built from, complementing the sustaining `adsr`. `env-follower` rectifies an audio input and
  smooths it with separate attack/release one-pole ballistics into a control signal that rides
  the input's loudness — the source for auto-wah (envelope → cutoff), dynamic tremolo and
  ducking (audio → CV). Unit-tested (AD — rise/fall to zero, release ignored, retrigger, idle;
  follower — tracks amplitude, louder→higher, decays on silence, attack faster than release)
  plus a registry test; regenerated [node reference](docs/nodes.md).
- **Sample & hold and slew-limiter nodes (`vb-nodes`, kinds `sample-hold` / `slew`)** — two
  modulation utilities. `sample-hold` latches its `in` value on each rising edge of a `trig`
  input and holds it between triggers — fed noise and a clock it gives stepped-random
  modulation, fed an LFO it quantises it into clocked stairs. `slew` bounds a control signal's
  rate of change with **separate rise and fall** limits (a generalised, asymmetric `glide`):
  portamento, rounded steps, lopsided ramps, and taming of abrupt CV jumps; zero times pass
  through unchanged. Unit-tested (S&H — holds until first trigger, latches on the edge,
  re-samples each edge; slew — step is rate-limited, rise/fall independent, zero passes
  through, primes to the first input) plus a registry test; regenerated
  [node reference](docs/nodes.md).
- **Ring modulator and bitcrusher effect nodes (`vb-nodes`, kinds `ring-mod` / `bitcrusher`)**
  — two effects broadening the effect category. `ring-mod` multiplies the input by an internal
  sine carrier (`freq_hz`) with a dry/wet `mix`, producing the inharmonic sum-and-difference
  sidebands of the classic ring modulator (metallic, bell, robot-voice) — self-contained,
  unlike the bare `mul` node. `bitcrusher` degrades the signal two ways: bit-depth
  quantisation to `2^bits` levels and sample-rate decimation (hold each sample for
  `downsample` steps), with a dry/wet `mix`. Unit-tested (ring-mod — dry passthrough at
  mix 0, bounded full-wet product that reshapes the signal, carrier frequency matters, param
  clamp; bitcrusher — dry passthrough, low bit depth collapses to few levels, downsampling
  holds runs of samples, param clamp) plus a registry test; regenerated
  [node reference](docs/nodes.md).
- **Math utility nodes: subtract, rectify, attenuvert (`vb-nodes`, kinds `sub` / `rectify` /
  `attenuvert`)** — three small stateless utilities filling out the math category. `sub` is
  `a - b` (the difference counterpart to `add`). `rectify` is `|in|` (full-wave rectifier —
  octave-up fuzz for audio, bipolar→unipolar for control). `attenuvert` is `in·scale + offset`,
  the workhorse CV utility: a negative `scale` inverts (subsuming a plain invert), `offset`
  shifts the centre, together mapping a modulator into the range a destination wants. Added to
  `vb-nodes/src/math.rs` alongside the existing gain/add/mul/mix/clamp; unit-tested and covered
  by a registry test; regenerated [node reference](docs/nodes.md).
- **Tremolo and chorus effect nodes (`vb-nodes`, kinds `tremolo` / `chorus`)** — two
  time-varying effects, each self-contained with its own internal LFO. `tremolo` scales the
  amplitude with a sine LFO (`rate_hz`, `depth`), sweeping the gain between unity and
  `1 - depth` — the vintage amp shimmer / pulsing pad. `chorus` mixes in one LFO-modulated
  delay tap (a fractional, linearly-interpolated delay line wobbling around ~12 ms by
  `depth_ms` at `rate_hz`), whose moving-tap Doppler detune thickens and widens the sound,
  balanced dry/wet by `mix`. Unit-tested (tremolo — depth-0 transparency, gain swings between
  1 and 1-depth on a DC input, faster rate → more troughs, depth clamp; chorus — dry
  passthrough, the delayed tap starts silent then sounds, wet reshapes the signal, silent
  stays silent, mix clamp) plus a registry test; regenerated [node reference](docs/nodes.md).
- **Pulse-width and hard-sync oscillator nodes (`vb-nodes`, kinds `pwm` / `hard-sync`)** — two
  more pitch-tracking generators. `pwm` is a PolyBLEP band-limited pulse with an adjustable
  duty cycle (`width`): 50% is a square, narrower widths thin the waveform toward a bright,
  nasal spike; both discontinuities are PolyBLEP-corrected so it stays alias-free. `hard-sync`
  runs a slave sawtooth at `sync_ratio ×` the master (played) pitch and hard-resets the slave's
  phase every master cycle — the tearing, formant-swept "sync lead"; at ratio 1 it is a plain
  saw. Unit-tested (pwm — 50% is a balanced square, narrow pulse is asymmetric, pitch tracking,
  width clamp; hard-sync — ratio 1 is a plain saw, higher ratio adds crossings, output stays
  periodic at the master pitch, pitch tracking + ratio clamp) plus a registry test; regenerated
  [node reference](docs/nodes.md).
- **Showcase example patches for the new nodes (`examples/`, 000160)** — three factory patches
  demonstrating the recent additions end-to-end in the real engine: `supersaw_lead.ron`
  (supersaw → envelope-swept ladder → note-gated VCA, a fat trance lead), `fm_ep.ron` (a
  two-operator FM electric piano — a `fm-op` modulator into a `fm-op` carrier's `mod` input,
  plucked by a no-sustain envelope), and `auto_wah.ron` (a built-in saw drone through a
  `tremolo` whose amplitude an `env-follower` tracks to sweep a `vcf` cutoff — the classic
  auto-wah). Each is wired into the `vb-plugin` examples manifest and covered by a targeted
  engine test (supersaw lead / FM EP silent at rest and sounding on a note; auto-wah drones
  audibly and stays bounded).
- **AudioUnit validation gate (`cargo xtask validate-au` + CI, 000161)** — a new xtask
  subcommand reads the built `.component`'s type/subtype/manufacturer codes from its
  `Info.plist` and runs Apple's `auval -v` on them — the authoritative macOS AudioUnit check
  (format negotiation, render at every sample rate, parameter setting, MIDI). A non-blocking CI
  `au validate` job builds the AUv2 via clap-wrapper and runs it, so the wrapped **AU export is
  proven in CI**, not just the CLAP (which the required `clap-validator` gate already covers).
  The `auval_args` argument builder is unit-tested; the job is advisory because the wrapped-AU
  build is heavy and `auval` is macOS-only.
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
- **Multiband distortion effect patch (`examples/maul.ron`, Epic 13 / 000142)** — an
  fxpansion-Maul-style three-band distortion, built from existing nodes: a rich saw split
  into low / mid / high by two `svf` crossovers (the low band is one filter's lowpass; the
  mid and high come from cascading the highpass into a second filter), each band driven by
  its own `waveshaper` (foldback / saturate / hard-clip) and recombined through a trim gain.
  An effect demo with a built-in source, so it sounds without a note; the examples test
  asserts the three-band structure and audible output.
- **Korg Minilogue XD factory clone (`examples/minilogue.ron`, Epic 13 / 000140)** — the
  first **polyphonic** factory patch, proving the voice-allocation system end to end. Four
  independent voice chains, each fed by a `voice-in` node bound to its own slot of the
  `PolyNoteSource` (which the plugin already drives through its `VoiceAllocator` on every
  note), are summed into a `reverb`. Each voice is two detuned saw VCOs plus a triangle
  "digital" oscillator → an envelope-swept `ladder` → a note-gated VCA. Polyphony needed no
  engine change — it is expressed in the graph as parallel voices. The examples test plays a
  three-note chord and asserts it sums to more energy than a single note (and stays silent
  at rest). The digital multi-engine is approximated with the triangle oscillator; a true
  wavetable oscillator is left as a future node.
- **Per-note gate node (`vb-nodes`, kind `key-gate`)** — emits `1.0` while a specific MIDI
  note is held in any voice, else `0.0` (backed by a new `PolyNoteSource::note_gate(note)`
  query). Because it reads the polyphonic note source, several keys can be held at once, so
  a drum kit can trigger kick + hat together off one note stream.
- **Drum-machine kit factory patch (`examples/drumkit.ron`, Epic 13 / 000141)** — a
  TR-style kit synthesised from existing nodes: a kick (a `vco` sine whose pitch drops via a
  fast envelope), a snare (`noise` + a tone), and a hat (highpassed `noise`), each fired by
  its own `key-gate` on notes 36 / 38 / 42 and summed. Silent at rest, each drum triggers on
  its note, and — because the gates are polyphonic — several drums sound together (all
  asserted in the examples test). **This completes Epic 13** (factory preset library): six
  factory patches ship — Minimoog Model D, Moog Minitaur, Moog Grandmother, Behringer
  Neutron, Korg Minilogue XD (polyphonic), a multiband distortion effect, and this kit.
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
