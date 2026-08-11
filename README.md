# VST_bomb

**Cross-platform no-code / low-code editor for VST/AU plugins.**

Build an audio plugin out of nodes in a visual editor (NI Reaktor level — or better)
and export a ready-to-use **VST3 / AU / CLAP** plugin. No coding required.

> Status: 🌱 project start. Backlog and plan — in the [Tasks](#-tasks-backlog) section below.

---

## 🎯 Goal

- Visual **DSP graph editor**: oscillators, filters, envelopes, math, MIDI/audio I/O —
  wire nodes together and hear the result in real time.
- **Export to plugins**: one project → `VST3`, `AU`, `CLAP`, plus a standalone app.
- UI **like Reaktor or better**: infinite canvas, nested macro-nodes, live
  meters/scopes, undo/redo, clean port typing.
- **WebUI in the future**: the same editor in the browser.

## 🧱 Stack & key decisions

Primary language — **Rust** (the client's preference, and the modern audio stack in
Rust is now up to the task), with a thin C++ build step only for the format wrapper.

| Layer | Technology | Why |
|-------|-----------|-----|
| DSP engine / runtime | **Rust** | memory safety in real-time audio, speed, one codebase for all platforms |
| Format export | **CLAP** via [`nih-plug`](https://github.com/robbert-vdh/nih-plug) | native from Rust, open format |
| VST3 / AU wrapper | [`clap-wrapper`](https://github.com/free-audio/clap-wrapper) | wraps CLAP → VST3 + AUv2 + standalone, **off-the-shelf, no code to write** |
| Audio I/O (standalone) | [`cpal`](https://github.com/RustAudio/cpal) | cross-platform audio output |
| Node editor UI | [`egui`](https://github.com/emilk/egui) | immediate-mode, cross-platform, **compiles to WASM → WebUI** |
| DSP primitives | `fundsp` / custom nodes | fast start on a base node library |

### Why not "pure C++/JUCE"

JUCE gives AU out of the box, but: GPL/commercial license, manual memory management in
C++ in a real-time context, and a harder WebUI path. The **Rust + CLAP +
`clap-wrapper`** combo covers VST3/AU/standalone while keeping all business logic in
safe Rust and opening the road to WASM.

### Export model

1. **MVP — "runtime + graph"** (like Reaktor `.ens`): the exported plugin = our DSP
   runtime + a serialized graph description. Quick to implement, one runtime.
2. **Later — AOT compilation** of the graph into optimized native code for minimal
   latency and CPU load.

## 🏛️ Principles

- **TDD** — tests are written first; no code is accepted without tests.
- **SOLID** — engine, nodes, UI and export are separated by clear boundaries
  (interfaces); nodes plug into the engine like extensions.
- **Cargo workspace** — independent crates, each with its own tests.

## 📦 Planned structure (Cargo workspace)

```
VST_bomb/
├─ crates/
│  ├─ vb-core/        # graph model, port types, scheduler, sample processing
│  ├─ vb-nodes/       # DSP node library (osc, filters, envelopes, math, MIDI)
│  ├─ vb-project/     # project serialization (graph format, save/load, versions)
│  ├─ vb-runtime/     # real-time graph execution (buffers, parameters, MIDI)
│  ├─ vb-editor/      # egui node editor (canvas, wires, inspector, undo)
│  ├─ vb-standalone/  # standalone app (cpal + editor)
│  └─ vb-plugin/      # CLAP export (nih-plug), VST3/AU build via clap-wrapper
├─ README.md          # project overview + task backlog
└─ LICENSE            # Apache-2.0
```

## 🚀 Quick start (grows as development proceeds)

```bash
# run the standalone editor
cargo run -p vb-standalone

# tests / lint / format (required before every commit)
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## 🗺️ Roadmap (epics)

1. Project infrastructure & CI
2. Core: DSP graph engine
3. Node library
4. Node editor (UI like Reaktor)
5. Project serialization
6. Standalone audio runtime
7. Plugin export (CLAP → VST3/AU)
8. Parameters, automation, presets
9. WebUI (future)
10. Documentation & release
11. Patch/preset library on drevo (optional)
12. AI assistant (planned from the start) — generate/edit patches from a description

> **The AI assistant is a cross-cutting feature, not a "late add-on."** The LLM
> provider abstraction and the text representation of the graph are laid down early
> (in parallel with the core), so the whole editor is designed for the human + assistant
> pair from the outset. Default models — Claude (Anthropic) behind the provider
> abstraction. The AI **proposes** edits; they are applied as ordinary undoable commands
> with preview — the engine stays the source of truth.

Detailed checklist — in the [Tasks](#-tasks-backlog) section.

---

## ✅ Tasks (backlog)

Format: `- [ ] NNNNNN — description`. Numbers are global and unique, order = rough
implementation order. Every task with code must include tests
(**TDD**: test → implementation → integration). Legend: 🧩 subtask · 🧪 tests are part of
the definition of done.

### Epic 1 — Project infrastructure & CI

- [x] 000001 — Initialize the Cargo workspace (`Cargo.toml` with members: `vb-core`, `vb-nodes`, `vb-project`, `vb-runtime`, `vb-editor`, `vb-standalone`, `vb-plugin`)
- [x] 000002 — Update `.gitignore` for Rust (`/target`, `Cargo.lock` policy, `*.rs.bk`)
- [x] 000003 — Set up `rustfmt.toml` and `clippy` (deny warnings) as the repo standard
- [x] 000004 — Set up CI (GitHub Actions): `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`
- [x] 000005 — CI matrix across OSes: Linux, macOS, Windows
- [x] 000006 — 🧩 Configure dependency caching in CI (faster builds)
- [x] 000007 — Add `CONTRIBUTING.md` with TDD/SOLID rules and branch workflow
- [x] 000008 — Crate stubs: each crate compiles and has an empty passing test 🧪
- [ ] 000009 — Set up `cargo-nextest` (optional) for fast test runs
- [ ] 000010 — Set up test coverage measurement (`cargo-llvm-cov`) in CI

### Epic 2 — Core: DSP graph engine (`vb-core`)

- [ ] 000011 — Design the data model: `Node`, `Port`, `Edge`, `Graph` (interfaces/traits, SOLID) 🧪
- [ ] 000012 — Port types and compatibility rules: `Audio`, `Control/CV`, `Event/MIDI` 🧪
- [ ] 000013 — `Node` trait (process contract, in/out, state) as the extension point 🧪
- [ ] 000014 — Add/remove nodes and connections with validation 🧪
- [ ] 000015 — Cycle detection in the graph (forbid invalid feedback loops) 🧪
- [ ] 000016 — Topological sort for processing order 🧪
- [ ] 000017 — Processing scheduler: traverse the graph over sample buffers 🧪
- [ ] 000018 — Audio buffer model (block size, channels, allocation outside the RT thread) 🧪
- [ ] 000019 — Node parameter model (ranges, units, defaults, normalization) 🧪
- [ ] 000020 — 🧩 Allowed feedback via an explicit delay node (feedback delay) 🧪
- [ ] 000021 — Handle sample rate / block size and their on-the-fly changes 🧪
- [ ] 000022 — Graph diagnostics: clear errors (incompatible ports, dangling inputs) 🧪
- [ ] 000023 — Core benchmarks (`criterion`) on typical graphs

### Epic 3 — Node library (`vb-nodes`)

- [ ] 000024 — Generator: oscillator (sine/saw/square/triangle) with anti-aliasing 🧪
- [ ] 000025 — Noise generator (white/pink) 🧪
- [ ] 000026 — Filter: biquad (LP/HP/BP/notch) 🧪
- [ ] 000027 — Filter: state-variable / ladder 🧪
- [ ] 000028 — ADSR envelope 🧪
- [ ] 000029 — LFO (with tempo sync) 🧪
- [ ] 000030 — Math nodes: add/mul/mix/gain/clamp 🧪
- [ ] 000031 — Routing nodes: splitter, merger, selector 🧪
- [ ] 000032 — Audio I/O nodes: plugin in/out (channels) 🧪
- [ ] 000033 — MIDI input: note on/off → gate/pitch/velocity 🧪
- [ ] 000034 — Polyphony / voices (voice allocation) 🧪
- [ ] 000035 — Effects: delay, reverb (basic) 🧪
- [ ] 000036 — Effect: distortion / waveshaper 🧪
- [ ] 000037 — 🧩 Node registry (registration + metadata for the UI: name, ports, category)
- [ ] 000038 — Macro-node (nested subgraph as a single node) — the foundation for the Reaktor style 🧪

### Epic 4 — Node editor / UI (`vb-editor`, egui)

- [ ] 000039 — `egui` window shell + infinite canvas (pan/zoom) 🧪
- [ ] 000040 — Node rendering (title, ports, body, selection) 🧪
- [ ] 000041 — Draw and create wires by drag-and-drop between ports 🧪
- [ ] 000042 — Connection validation while dragging (highlight compatible ports) 🧪
- [ ] 000043 — Node palette/search and adding to the canvas from the registry 🧪
- [ ] 000044 — Property inspector for the selected node (edit parameters) 🧪
- [ ] 000045 — Select, move, delete, copy/paste nodes 🧪
- [ ] 000046 — Undo/redo (command stack) 🧪
- [ ] 000047 — Enter a macro-node and navigate the subgraph hierarchy 🧪
- [ ] 000048 — Live visualizations: level meter, oscilloscope, spectrum
- [ ] 000049 — Control widgets: knob, slider, switch, XY-pad (mapped to parameters) 🧪
- [ ] 000050 — Grid/alignment/auto-layout of nodes
- [ ] 000051 — Theme and "better than Reaktor" styling (visual polish)

### Epic 5 — Project serialization (`vb-project`)

> Layer separation: **a single exported patch** = a flat RON file (embedded into the
> plugin, no DB dependency). The editor's **patch/preset/module library** = optional
> `drevo` (see Epic 11). RON is the source of truth; drevo stores links/metadata and
> does not replace the project file.

- [ ] 000052 — Project file format (graph + node positions + parameters): **RON** via `serde` (decided) 🧪
- [ ] 000053 — Save/Load a project with round-trip tests 🧪
- [ ] 000054 — Format versioning and migrations 🧪
- [ ] 000055 — Export/import a single macro-node as a reusable module 🧪
- [ ] 000056 — Validate a loaded file (robustness to corrupt/foreign data) 🧪

### Epic 6 — Standalone audio runtime (`vb-runtime`, `vb-standalone`)

- [ ] 000057 — RT graph executor: no allocations/locks in the audio thread 🧪
- [ ] 000058 — Double buffering / thread-safe delivery of graph edits into the RT thread 🧪
- [ ] 000059 — `cpal` integration: audio output to the default device
- [ ] 000060 — Choose audio device, sample rate, buffer size in the UI
- [ ] 000061 — MIDI input from a device (`midir`) in standalone
- [ ] 000062 — Wire editor + runtime together: graph changes are "audible" in real time
- [ ] 000063 — Monitor CPU load and xruns 🧪

### Epic 7 — Plugin export (`vb-plugin`: CLAP → VST3/AU)

- [ ] 000064 — `nih-plug` plugin scaffold, **CLAP** export, load the serialized graph
- [ ] 000065 — Route plugin parameters (host automation) into graph parameters 🧪
- [ ] 000066 — Handle MIDI/notes from the host in the plugin runtime 🧪
- [ ] 000067 — Embed the GUI (egui editor or a simplified run-view) into the plugin window
- [ ] 000068 — Save/restore plugin state in the host (preset state) 🧪
- [ ] 000069 — Integrate `clap-wrapper`: build **VST3** from CLAP
- [ ] 000070 — Integrate `clap-wrapper`: build **AUv2** (macOS) from CLAP
- [ ] 000071 — Build the standalone app via `clap-wrapper`
- [ ] 000072 — Export script/`xtask`: project graph → ready plugin bundle in one command
- [ ] 000073 — Plugin validation: `clap-validator`, `pluginval` (VST3/AU) in CI
- [ ] 000074 — Test loading the exported plugin in a real host (Reaper/Logic) — checklist
- [ ] 000075 — Sign/notarize the AU bundle on macOS (document the process)

### Epic 8 — Parameters, automation, presets

- [ ] 000076 — Mapping system: UI widget ↔ graph parameter ↔ host parameter 🧪
- [ ] 000077 — Parameter smoothing (declick/ramp) in RT 🧪
- [ ] 000078 — Presets: save/load a set of parameter values 🧪
- [ ] 000079 — Preset bank and switching from UI/host 🧪
- [ ] 000080 — Modulation matrix (sources → destinations) 🧪

### Epic 9 — WebUI (future)

- [ ] 000081 — Build `vb-editor` to **WASM** (egui/eframe web target) — proof of concept
- [ ] 000082 — Edit the graph in the browser (no audio) with serialization
- [ ] 000083 — Audio prototype in the browser via WebAudio/AudioWorklet (engine in WASM)
- [ ] 000084 — Define the web/native runtime boundary and a shared project format

### Epic 10 — Documentation, examples, release

- [ ] 000085 — User guide: build your first plugin (osc→filter→envelope→out)
- [ ] 000086 — Node reference (auto-generated from the registry: categories, ports, parameters)
- [ ] 000087 — A set of example projects (synth, delay effect, LFO modulation)
- [ ] 000088 — Developer guide: how to add your own node (`Node` trait + registration) 🧪
- [ ] 000089 — Screencast/GIF of the UI for the README
- [ ] 000090 — Set up releases: build VST3/AU/standalone bundles for all OSes in CI
- [ ] 000091 — Versioning and CHANGELOG
- [ ] 000092 — MVP release: working export of a minimal synth to VST3 + AU

### Epic 11 — Patch/preset library on drevo (optional)

> [drevo](https://github.com/ice1x/drevo) — an embedded graph DB in Rust (redb),
> cross-platform + WASM. Used **not** for the runtime DSP graph, but as the editor's
> "knowledge store": a catalog of patches, macro-nodes and presets with search and links.
> Enabled behind a feature flag so the core/export don't depend on a DB.
>
> **Integration — embedded crate via the direct Rust API (`src/db.rs`); Bolt/server is
> NOT used.** The drevo core already provides natively: FTS/BM25 (`search_fts_ranked`),
> vector ANN search on HNSW (`build_vector_index`), and auto-embedding (`semantic_register`
> / `set_embedder`) — all in-process, no socket/async runtime, preserving the WASM path
> (`wasm.rs`) and mobile (C-FFI `drevo.h`). Bolt/Cypher are optional frontends over the
> same core, needed only for networked/remote clients
> (ref: drevo [#202](https://github.com/ice1x/drevo/issues/202),
> [#251](https://github.com/ice1x/drevo/issues/251), [#48](https://github.com/ice1x/drevo/issues/48)).

- [ ] 000093 — Integration assessment: add `drevo` as a dependency behind the `library` feature flag, CRUD smoke test 🧪
- [ ] 000094 — Storage schema: `Patch` / `MacroNode` / `Preset` nodes, `uses` / `derived-from` / `tagged` edges 🧪
- [ ] 000095 — Import a patch (RON) → write metadata and links into drevo (RON stays the source of truth) 🧪
- [ ] 000096 — Full-text search over the library (name, tags, description) via drevo's FTS/BM25 🧪
- [ ] 000097 — UI: library browser in the editor (list/search/filter by tags, insert a macro-node onto the canvas)
- [ ] 000098 — Store preset banks and "preset → patch" links in drevo 🧪
- [ ] 000099 — 🧩 Verify the `library` layer builds to WASM (shared path with Epic 9)
- [ ] 000100 — Library export/backup and drevo schema migrations 🧪

### Epic 12 — AI assistant (planned from the start)

> A cross-cutting feature: the no-code editor is designed from the outset for the
> human + assistant pair. The foundation (000101–000102) is laid early, in parallel with
> the core. Default models — **Claude (Anthropic)** behind a provider abstraction.
> Principle: **AI proposes — the user applies**; edits go through undoable commands with
> preview, the engine (`vb-core`) stays the source of truth and validator.
>
> **The assistant's knowledge base = the same embedded drevo** (vector+FTS+auto-embedding,
> direct Rust API `db.rs`), not a separate vector store. Access is hidden behind a
> `KnowledgeStore` abstraction (SOLID/testability); the sole production implementation is
> drevo. Retrieval (000109) is switched on **after** drevo is shaken out in Epic 11.

- [ ] 000101 — 🏗️ LLM provider abstraction (`AiProvider` trait), Claude by default, model/key config 🧪
- [ ] 000102 — 🏗️ Text/JSON representation of the graph for LLM context (prompt serialization, round-trip) 🧪
- [ ] 000103 — NL → patch: generate a graph from a description, mandatory validation of the result via `vb-core` 🧪
- [ ] 000104 — NL edit of an existing graph: the assistant returns a diff, preview before applying 🧪
- [ ] 000105 — Apply assistant edits as undoable commands (shared stack with manual editing) 🧪
- [ ] 000106 — Suggestions: next node, auto-fix incompatible/dangling connections 🧪
- [ ] 000107 — Explain a patch/node in natural language (chat panel / tooltips)
- [ ] 000108 — UI: assistant chat panel in the editor (request → proposal → apply/reject)
- [ ] 000109 — RAG over the drevo library: example patches/presets as context (ties into Epic 11) 🧪
- [ ] 000110 — Privacy/security: API key handling, explicit consent before sending the project, offline mode 🧪
- [ ] 000111 — 🧩 Offline/local models (optional) via the same `AiProvider` abstraction

**Maintaining the list:** mark `[x]` when done, do not reuse numbers; add new tasks at
the end with a new global number (000112, …); split a large task into 🧩 subtasks with
their own numbers.

## 📄 License

Apache-2.0 — see [`LICENSE`](LICENSE).
