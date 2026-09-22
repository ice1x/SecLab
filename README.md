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

---

## 🤖 MVP feature: AI assistant that builds your synth/plugin with you

**The headline MVP scenario.** The user brings their own LLM API keys, describes in a
chat what device they want — the algorithm, what connects to what — gets honest,
grounded critique back, and after a short dialogue walks away with a working **VST/AU**
built from a validated patch graph. The assistant is a *collaborator with taste*, not a
code generator: it argues, simplifies, and only then builds.

### User flow

```
add API key ──▶ describe the device ──▶ critique & clarify ──▶ propose graph
   (once)        "I want a mono bass      assistant pushes        diff + preview
                  synth: saw → ladder     back: what's the        on the canvas
                  filter, LFO on          filter envelope?
                  cutoff, glide"          glide time? why 2
                                          LFOs — one is enough
                        ▲                                             │
                        └──────────── iterate in chat ◀───────────────┘
                                                                      │ accept
                                                                      ▼
                                                    validate (vb-core) ──▶ audition
                                                    live in the editor ──▶ export
                                                    VST3 / AU / CLAP bundle
```

### Design pillars

1. **BYOK — bring your own keys.** The user adds keys for their LLM provider(s) in
   Settings. Keys live in the OS keychain (never in project files, never in telemetry).
   `AiProvider` trait; Anthropic (Claude) is the reference implementation, an
   OpenAI-compatible adapter widens coverage, local models can slot in later (000111).
   Nothing is sent anywhere until the user explicitly starts a chat, and the exact
   payload is inspectable ("what will be sent?" button).
2. **Grounded, not hallucinated.** The assistant works from *distilled documentation*:
   an auto-generated node reference (from the registry: kinds, ports, parameters,
   categories — 000086) compressed into a compact "node card" corpus that fits the
   context window. It can only propose nodes that exist; the schema it emits is the
   same RON `ProjectDoc` the editor loads, so `vb-core` validation is the hard gate —
   an invalid proposal never reaches the canvas, it goes back to the model with the
   validator's issue list until it converges or the assistant admits defeat.
   *Beyond MVP, grounding upgrades from a static card corpus to a **knowledge graph in
   drevo**: the product docs are converted into typed ontology (node kinds, ports,
   compatibility rules, DSP heuristics, example patches, linked by `has_port` /
   `compatible_with` / `explains` edges), and the assistant retrieves only the slice a
   question needs via `KnowledgeStore` — GraphRAG instead of prompt-stuffing, saving
   tokens per exchange. The same graph doubles as a **drevo MCP server** for external
   AI agents (dev tooling) — that server mode is an optional frontend and does not
   change the in-app rule: embedded drevo via the direct Rust API.*
3. **Skills & pre-prompts.** Reusable prompt assets shipped with the app: a system
   pre-prompt (role, output contract, refusal rules), per-task *skills* (synth design,
   effect design, patch explanation, patch repair), and DSP design heuristics (gain
   staging, aliasing, feedback safety). Stored as versioned files in the repo, unit
   tested with golden transcripts, and user-extensible.
4. **Sound critique, Occam's razor.** Before building anything the assistant runs a
   critic pass over the user's request *and* over its own draft: is every node earning
   its place? Can two oscillators be one? Is the modulation routing audible at all?
   The critique is shown to the user as short bullet points with trade-offs — the user
   decides, the assistant argues but never overrules.
5. **Context compression / distillation.** Long dialogues and big patches must not blow
   the context window or the user's token budget: the graph is summarized structurally
   (node counts, chains, modulation routes) instead of dumped verbatim; earlier turns
   are distilled into a running "session brief"; node cards are pre-distilled offline.
   Budget guard: the assistant reports approximate tokens/cost per exchange.
6. **Dialogue → artifact.** The end state of a successful conversation is not text —
   it is a patch on the canvas the user can audition immediately, plus (on request) the
   exported VST3/AU/CLAP bundle via the standard export pipeline (Epic 7). Every
   assistant edit is an undoable command in the same history as manual edits.

### MVP cut line

**In:** one provider (Claude) + OpenAI-compatible adapter behind `AiProvider`; keychain
key storage; chat panel; distilled node reference; system pre-prompt + 2 skills
("design a synth", "explain/repair a patch"); critic pass; propose→validate→repair
loop; apply-as-undoable-diff; audition; export via Epic 7.
**Out (post-MVP):** multi-agent orchestration, RAG over the drevo patch library
(000109), local models (000111), auto-tuning parameters by ear (analysis-driven),
voice input.

Detailed checklist — in the [Tasks](#-tasks-backlog) section.

---

## 📚 Documentation

- [Build your first plugin](docs/first-plugin.md) — a getting-started walkthrough
  (oscillator → filter → envelope → output).
- [Adding your own node](docs/adding-a-node.md) — extend the DSP library with custom nodes.
- [Node reference](docs/nodes.md) — every built-in node, its ports and parameters
  (auto-generated).
- [CHANGELOG](CHANGELOG.md) — notable changes and the versioning policy.
- [`examples/`](examples) — small, tested starter patches.

**Running the tests:** `cargo test --workspace`, or — faster and with clearer output —
[`cargo nextest run`](https://nexte.st/) (config in [`.config/nextest.toml`](.config/nextest.toml);
it auto-retries a couple of load-sensitive timing tests). Nextest does not run doctests;
use `cargo test --doc` for those. **Coverage:** `cargo llvm-cov --workspace --summary-only`
([cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)); CI runs it as a non-blocking
advisory job.

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
- [x] 000009 — Set up `cargo-nextest` (optional) for fast test runs — [`.config/nextest.toml`](.config/nextest.toml) (default + `ci` profiles; targeted auto-retries for the load-sensitive timing tests)
- [x] 000010 — Set up test coverage measurement (`cargo-llvm-cov`) in CI — non-blocking advisory `coverage` job in [ci.yml](.github/workflows/ci.yml) (baseline ~85% line coverage)

### Epic 2 — Core: DSP graph engine (`vb-core`)

- [x] 000011 — Design the data model: `Node`, `Port`, `Edge`, `Graph` (interfaces/traits, SOLID) 🧪
- [x] 000012 — Port types and compatibility rules: `Audio`, `Control/CV`, `Event/MIDI` 🧪
- [x] 000013 — `Node` trait (process contract, in/out, state) as the extension point 🧪
- [x] 000014 — Add/remove nodes and connections with validation 🧪
- [x] 000015 — Cycle detection in the graph (forbid invalid feedback loops) 🧪
- [x] 000016 — Topological sort for processing order 🧪
- [x] 000017 — Processing scheduler: traverse the graph over sample buffers 🧪
- [x] 000018 — Audio buffer model (block size, channels, allocation outside the RT thread) 🧪
- [x] 000019 — Node parameter model (ranges, units, defaults, normalization) 🧪
- [x] 000020 — 🧩 Allowed feedback via an explicit delay node (feedback delay) 🧪
- [x] 000021 — Handle sample rate / block size and their on-the-fly changes 🧪
- [x] 000022 — Graph diagnostics: clear errors (incompatible ports, dangling inputs) 🧪
- [x] 000023 — Core benchmarks (`criterion`) on typical graphs

### Epic 3 — Node library (`vb-nodes`)

- [x] 000024 — Generator: oscillator (sine/saw/square/triangle) with anti-aliasing 🧪
- [x] 000025 — Noise generator (white/pink) 🧪
- [x] 000026 — Filter: biquad (LP/HP/BP/notch) 🧪
- [x] 000027 — Filter: state-variable / ladder 🧪
- [x] 000028 — ADSR envelope 🧪
- [x] 000029 — LFO (with tempo sync) 🧪
- [x] 000030 — Math nodes: add/mul/mix/gain/clamp 🧪
- [x] 000031 — Routing nodes: splitter, merger, selector 🧪
- [x] 000032 — Audio I/O nodes: plugin in/out (channels) 🧪
- [x] 000033 — MIDI input: note on/off → gate/pitch/velocity 🧪
- [x] 000034 — Polyphony / voices (voice allocation) 🧪
- [x] 000035 — Effects: delay, reverb (basic) 🧪
- [x] 000036 — Effect: distortion / waveshaper 🧪
- [x] 000037 — 🧩 Node registry (registration + metadata for the UI: name, ports, category)
- [x] 000038 — Macro-node (nested subgraph as a single node) — the foundation for the Reaktor style 🧪
- [x] 000131 — VCA node (control-modulated amplifier `out = in·(bias + amount·cv)`): the first CV-consuming node, so an LFO/envelope can actually modulate audio (unblocks the modulation matrix and the tremolo example) 🧪
- [x] 000132 — VCF node (voltage-controlled filter): a biquad whose cutoff a control input sweeps (`cutoff = base_hz + amount_hz·cv`) — the README MVP's "LFO on cutoff", with a live filter-sweep example 🧪
- [x] 000133 — VCO node (control-modulated oscillator pitch): an oscillator whose frequency a control input sweeps exponentially (`freq = base_hz·2^(amount_oct·cv)`, 1 V/oct) — the pitch leg of modulation alongside VCA/VCF, so an LFO/envelope can vibrato or sweep pitch, with a live vibrato example 🧪

### Epic 4 — Node editor / UI (`vb-editor`, egui)

- [x] 000039 — `egui` window shell + infinite canvas (pan/zoom) 🧪
- [x] 000040 — Node rendering (title, ports, body, selection) 🧪
- [x] 000041 — Draw and create wires by drag-and-drop between ports 🧪
- [x] 000042 — Connection validation while dragging (highlight compatible ports) 🧪
- [x] 000043 — Node palette/search and adding to the canvas from the registry 🧪
- [x] 000044 — Property inspector for the selected node (edit parameters) 🧪
- [x] 000045 — Select, move, delete, copy/paste nodes 🧪
- [x] 000046 — Undo/redo (command stack) 🧪
- [x] 000047 — Enter a macro-node and navigate the subgraph hierarchy 🧪
- [x] 000048 — Live visualizations: level meter, oscilloscope, spectrum 🧪
- [x] 000049 — Control widgets: knob, slider, switch, XY-pad (mapped to parameters) 🧪
- [x] 000050 — Grid/alignment/auto-layout of nodes 🧪
- [x] 000051 — Theme and "better than Reaktor" styling (visual polish)

### Epic 5 — Project serialization (`vb-project`)

> Layer separation: **a single exported patch** = a flat RON file (embedded into the
> plugin, no DB dependency). The editor's **patch/preset/module library** = optional
> `drevo` (see Epic 11). RON is the source of truth; drevo stores links/metadata and
> does not replace the project file.

- [x] 000052 — Project file format (graph + node positions + parameters): **RON** via `serde` (decided) 🧪
- [x] 000053 — Save/Load a project with round-trip tests 🧪
- [x] 000054 — Format versioning and migrations 🧪
- [x] 000055 — Export/import a single macro-node as a reusable module 🧪
- [x] 000056 — Validate a loaded file (robustness to corrupt/foreign data) 🧪

### Epic 6 — Standalone audio runtime (`vb-runtime`, `vb-standalone`)

- [x] 000057 — RT graph executor: no allocations/locks in the audio thread 🧪
- [x] 000058 — Double buffering / thread-safe delivery of graph edits into the RT thread 🧪
- [x] 000059 — `cpal` integration: audio output to the default device
- [x] 000060 — Choose audio device, sample rate, buffer size in the UI 🧪
- [x] 000061 — MIDI input from a device (`midir`) in standalone 🧪
- [x] 000062 — Wire editor + runtime together: graph changes are "audible" in real time 🧪
- [x] 000063 — Monitor CPU load and xruns 🧪

### Epic 7 — Plugin export (`vb-plugin`: CLAP → VST3/AU)

- [x] 000064 — `nih-plug` plugin scaffold, **CLAP** export, load the serialized graph 🧪
- [x] 000065 — Route plugin parameters (host automation) into graph parameters 🧪
- [x] 000066 — Handle MIDI/notes from the host in the plugin runtime 🧪
- [x] 000067 — Embed the GUI (egui editor or a simplified run-view) into the plugin window — egui run-view behind the `gui` feature (`nih_plug_egui`): shows the loaded patch (nodes by kind, edges) and live parameter values; wrapped bundles build with it, `validate()`/CI clap-validator stay editor-free 🧪
- [x] 000068 — Save/restore plugin state in the host (preset state) 🧪
- [x] 000069 — Integrate `clap-wrapper`: build **VST3** from CLAP 🧪
- [x] 000070 — Integrate `clap-wrapper`: build **AUv2** (macOS) from CLAP 🧪
- [x] 000071 — Build the standalone app via `clap-wrapper` 🧪
- [x] 000072 — Export script/`xtask`: project graph → ready plugin bundle in one command 🧪
- [x] 000073 — Plugin validation: `clap-validator`, `pluginval` (VST3/AU) in CI *(clap-validator live in CI; pluginval joins when VST3/AU exports land — 000069/000070)*
- [x] 000074 — Test loading the exported plugin in a real host (Reaper/Logic) — checklist
- [x] 000075 — Sign/notarize the AU bundle on macOS (document the process)

### Epic 8 — Parameters, automation, presets

- [x] 000076 — Mapping system: UI widget ↔ graph parameter ↔ host parameter 🧪
- [x] 000077 — Parameter smoothing (declick/ramp) in RT 🧪
- [x] 000078 — Presets: save/load a set of parameter values 🧪
- [x] 000079 — Preset bank and switching from UI/host 🧪
- [x] 000080 — Modulation matrix (sources → destinations) 🧪
- [x] 000188 — 🐛 **Live parameter updates (no rebuild click)** — turning a knob during playback
  corrupted the sound: every parameter change (a knob move or automation) rebuilt the **whole**
  engine and swapped it in, resetting each processor's DSP state (oscillator phase, filter memory,
  envelope stage) — a click per rebuild, ~60/s while dragging. Parameters now update the **running**
  engine in place. `Processor::set_params` (default no-op → `false`) writes a node's fields without
  touching its state; `RtEngine::set_node_params` / `set_doc_params` (with `alias_doc_ids` mapping
  document ids, from the builder) route an update by id; the plugin's `process` applies each changed
  node's values live on the audio thread (allocation-free, gathered on the stack) and only rebuilds
  for a node without live support or a structural edit. Implemented for every param-bearing node in
  the default voice (ladder, pitch-osc, attenuvert/cv-attenuvert, adsr, vca, glide, lfo). Tested:
  `set_params` retunes the ladder while keeping its filter memory (vs `reset`), the default-patch
  kinds all report live support (others fall back), the engine routes a live update by node id, and
  a default-patch node updates by its document id. 🧪

### Epic 9 — WebUI (future)

- [x] 000081 — Build `vb-editor` to **WASM** (egui/eframe web target) — proof of concept
- [x] 000082 — Edit the graph in the browser (no audio) with serialization
- [x] 000083 — Audio prototype in the browser via WebAudio/AudioWorklet (engine in WASM) — the nih-plug-free [`vb-engine`](crates/vb-engine) core compiles to wasm; a runnable AudioWorklet demo + headless Node smoke test live in [`web/audio`](web/audio) 🧪
- [x] 000084 — Define the web/native runtime boundary and a shared project format

### Epic 10 — Documentation, examples, release

- [x] 000085 — User guide: build your first plugin (osc→filter→envelope→out) — [docs/first-plugin.md](docs/first-plugin.md), backed by the tested `examples/first_synth.ron` patch
- [x] 000086 — Node reference (auto-generated from the registry: categories, ports, parameters)
- [x] 000087 — A set of example projects (synth, delay effect, dual-osc unison, LFO tremolo, LFO filter sweep) 🧪
- [x] 000088 — Developer guide: how to add your own node (`Processor` trait + registration) — [docs/adding-a-node.md](docs/adding-a-node.md), with a compile-tested example in `crates/vb-nodes/tests/custom_node.rs` 🧪
- [ ] 000089 — Screencast/GIF of the UI for the README
- [ ] 000090 — Set up releases: build VST3/AU/standalone bundles for all OSes in CI — **macOS leg wired**: [`.github/workflows/release.yml`](.github/workflows/release.yml) builds the CLAP/VST3/AU/standalone bundles on a `v*` tag (or manual dispatch) via the existing `cargo xtask` commands, zips them, and uploads a `VST_bomb-macos-bundles` artifact; code-signing/notarization run only when the `DEVELOPER_ID`/`NOTARY_PROFILE` secrets are set (see [docs/releasing.md](docs/releasing.md)). Linux/Windows legs await the runner-billing fix (blocking `cross-platform.yml` too)
- [x] 000091 — Versioning and CHANGELOG — [CHANGELOG.md](CHANGELOG.md) (Keep a Changelog; documents the crate SemVer vs. the separate project format version), guarded by a test tying it to `vb_project::FORMAT_VERSION` 🧪
- [x] 000092 — MVP release: working export of a minimal synth to VST3 + AU — `cargo xtask bundle-vst3` / `bundle-auv2` wrap the CLAP into loadable VST3/AU bundles carrying the default synth patch; the AU passes Apple's `auval` in CI (`au validate` job) and the VST3/CLAP pass `clap-validator`; the release workflow packages both (see [docs/releasing.md](docs/releasing.md)) 🧪

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

- [x] 000093 — Integration assessment: add `drevo` as a dependency behind the `library` feature flag, CRUD smoke test — new `vb-library` crate; drevo embedded via its direct Rust API (`redb-backend`, no Bolt), opt-in `--features library`; CRUD + JSON-properties smoke test green 🧪
- [x] 000094 — Storage schema: `Patch` / `MacroNode` / `Preset` nodes, `uses` / `derived-from` / `tagged` edges — typed `Library` layer over drevo (`vb-library/src/library.rs`) with `kind`/`rel` vocabularies and kind-scoped traversals 🧪
- [x] 000095 — Import a patch (RON) → write metadata and links into drevo (RON stays the source of truth) — `import_patch(ProjectDoc)` stores summary metadata (node/edge counts, kinds, format version) + `tagged` links to `kind:*` facets; `patches_with_node_kind` queries them 🧪
- [x] 000096 — Full-text search over the library (name, tags, description) via drevo's FTS/BM25 — `Library::search` / `search_patches` (BM25, trigram); patch descriptions stored in the FTS-indexed body 🧪
- [x] 000097 — UI: library browser in the editor (list/search/filter by tags, insert a macro-node onto the canvas) — `LibraryBrowser` view-model (`vb-editor/src/library.rs`: case-insensitive name search + AND tag facets + `all_tags` chips) rendered by `library_browser_ui` (`library_panel.rs`) in the palette's `library` section; clicking an entry calls `import_module_by_name` to drop the saved macro onto the canvas. Items load from `modules_dir` (`refresh_library`) and reload keeps the live filters. Covered end-to-end headlessly: disk→list→filter, import→macro node on canvas (`node_count +1`, `is_macro`, selected), full-frame render with the section 🧪
- [x] 000098 — Store preset banks and "preset → patch" links in drevo — `add_preset_for` (values + `derived-from` link), `create_bank`/`add_to_bank` (`contains`), and `presets_for_patch`/`patch_of_preset`/`presets_in_bank` queries 🧪
- [x] 000099 — 🧩 Verify the `library` layer builds to WASM (shared path with Epic 9)
- [x] 000100 — Library export/backup and drevo schema migrations — `Library::export_json`/`import_json` (idempotent JSON snapshots) + `migrate` (up/down) pass-through 🧪

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

- [x] 000101 — 🏗️ LLM provider abstraction (`AiProvider` trait), Claude by default, model/key config 🧪
- [x] 000102 — 🏗️ Text/JSON representation of the graph for LLM context (prompt serialization, round-trip) 🧪
- [x] 000103 — NL → patch: generate a graph from a description, mandatory validation of the result via `vb-core` 🧪
- [x] 000104 — NL edit of an existing graph: the assistant returns a diff, preview before applying 🧪
- [x] 000105 — Apply assistant edits as undoable commands (shared stack with manual editing) 🧪
- [x] 000106 — Suggestions: next node, auto-fix incompatible/dangling connections 🧪
- [x] 000107 — Explain a patch/node in natural language (chat panel / tooltips)
- [x] 000108 — UI: assistant chat panel in the editor (request → proposal → apply/reject) — `assistant_panel_ui` over the `EditSession` view-model, behind the native-only `assistant` feature (keeps the provider stack out of wasm/plugin); **wired into `EditorApp`** (palette "assistant" section): a live `EditSession` seeded from the patch, a provider slot (`set_assistant_provider`), Submit → propose → diff, and Apply pushing the result onto the canvas. The provider call runs on a **background worker thread** (Submit snapshots the doc + spawns `edit_patch`; each frame polls the worker and records the proposal on the main thread via `EditSession::record`), so the blocking transport never freezes the egui frame — a "thinking…" spinner shows while it runs. Real backends are behind the `assistant-net` feature: `set_assistant_anthropic` / `set_assistant_openai` (BYOK key from the OS keychain) and `set_assistant_local` (Ollama); the base `assistant` feature stays HTTP/keychain-free and deterministically tested. The **Settings panel** (`settings_panel_ui` over `AssistantSettings`) is wired into the assistant section — provider/model/sampling, offline/consent, and a masked BYOK-key field over a `KeyStore` (OS keychain under `assistant-net`, in-session otherwise); "apply / connect" (re)builds the live provider the panel selected, and the worker snapshots the panel's config each request 🧪
- [x] 000109 — RAG over the drevo library: example patches/presets as context (ties into Epic 11) — `Library::retrieve_patches` / `retrieve_context` (FTS-ranked patches rendered as a compact prompt block) 🧪
- [x] 000110 — Privacy/security: API key handling, explicit consent before sending the project, offline mode; enforced in the editor's assistant path — a Submit is refused (no worker spawned) while offline or without consent, per `PrivacySettings::blocked_reason` 🧪
- [x] 000111 — 🧩 Offline/local models (optional) via the same `AiProvider` abstraction

#### MVP: "dialogue → your VST/AU" pipeline (see [MVP feature](#-mvp-feature-ai-assistant-that-builds-your-synthplugin-with-you))

- [x] 000112 — BYOK key management: add/remove provider API keys in Settings, stored in the OS keychain (never in project files); framework-free `AssistantSettings` view-model composes provider/model/sampling + privacy (offline/consent) + live key-presence and a `SendBlocker` readiness check (the key value flows straight to the `KeyStore`, never retained), ready for a Settings screen to bind to 🧪
- [x] 000113 — 🧩 `AiProvider` adapter: Anthropic (Claude) reference implementation (chat + structured output) 🧪
- [x] 000114 — 🧩 `AiProvider` adapter: OpenAI-compatible endpoint (covers most third-party providers) — `AssistantSettings` carries a base-URL override (`endpoint`), surfaced as a Settings field for the OpenAI-compatible provider and applied via `OpenAiProvider::with_url`, so it targets local/third-party servers (Ollama's shim, Together, Groq, a self-hosted gateway) rather than only api.openai.com 🧪
- [x] 000115 — Distilled node reference: auto-generate compact "node cards" from the registry (kinds, ports, params) as LLM context 🧪
- [x] 000116 — System pre-prompt & output contract: assistant role, RON `ProjectDoc` schema, refusal rules — versioned prompt assets in-repo 🧪
- [x] 000117 — Skill: "design a synth/effect from a description" (guided questions → draft graph) 🧪
- [x] 000118 — Skill: "explain / repair an existing patch" (uses validator issues as input) 🧪
- [x] 000119 — Critic pass (Occam's razor): review request + draft for redundant nodes/routing, surface trade-offs as short bullets 🧪
- [x] 000120 — Propose→validate→repair loop: every proposal gated by `vb-core`/`vb-project` validation, issues fed back to the model until convergence (with retry cap) 🧪
- [x] 000121 — Context compression: structural graph summary + running "session brief" distillation of earlier turns 🧪
- [x] 000122 — 🧩 Token/cost budget guard: estimate and report per-exchange usage, warn before large sends 🧪
- [x] 000123 — "What will be sent?" inspector: show the exact outgoing payload before the first request of a session
- [x] 000124 — Wire the accepted proposal into the editor: apply as undoable diff → audition in the editor (builds on 000104/000105)
- [x] 000125 — End-to-end MVP scenario test: scripted dialogue (mock provider) → validated graph → exported CLAP/VST3 bundle (ties into Epic 7) 🧪
- [x] 000126 — 🧩 Golden-transcript tests for prompt assets: pre-prompt/skills regressions caught by fixture dialogues 🧪

#### Knowledge graph grounding (docs → drevo → GraphRAG / MCP)

- [x] 000127 — Docs→graph pipeline: convert the product documentation (node reference from 000086/000115, port compatibility rules, DSP heuristics, example patches) into a typed drevo knowledge graph — `NodeKind` / `Port` / `Parameter` / `Concept` / `Heuristic` / `ExamplePatch` nodes with `has_port` / `compatible_with` / `in_category` / `explains` / `uses` edges — `Library::build_registry_ontology` derives the structural core (node-kind/port/parameter/category + has_port/has_parameter/in_category/compatible_with) from the registry; `add_concept`/`add_heuristic`/`link_explains` attach authored knowledge 🧪
- [x] 000128 — Assistant retrieval over the graph: `KnowledgeStore` queries fetch only the ontology slice a question needs (FTS + vector + graph traversal), replacing the full node-card corpus in the prompt; measure the token savings per exchange — `Library::knowledge_slice` (FTS → owning node-kinds → compact cards via traversal); test asserts a slice is a fraction of the full `node_reference` corpus 🧪
- [ ] 000129 — 🧩 Expose the same knowledge graph via drevo's MCP/server mode for external AI agents (dev tooling); off by default, does not change the embedded in-app path
- [x] 000130 — 🧩 Keep the graph in sync: regenerate from the registry/docs on release, stale-graph diff check in CI — `Library::missing_ontology_kinds(registry)` diff (empty = in sync); CI test asserts a fresh build covers every registry kind 🧪

### Epic 13 — Factory preset library (clones of real gear)

Recognisable factory patches so a user hears something musical the moment the plugin loads
and has real starting points to tweak — mono synths, polysynths, drum kits, effects. Many
depend on DSP nodes we don't have yet (ladder filter, sub-oscillator, polyphony, spring
reverb, noise, drum voices); those are noted per item and spawn their own 🧩 node subtasks
as each patch is picked up. All of them depend on 000134 (patches must carry parameter
values), so a preset can pin exact knob settings, not just a graph.

- [x] 000134 — 🧩 **Patches carry per-node parameter values** — a preset is structure **and**
  knob settings, not just a graph. Enabler for every factory patch below and for a
  silent-until-note default. `NodeEntry` now carries an optional `values` list (registry
  `params()` order) that round-trips through RON; the plugin's engine builder honours a
  node's stored values unless host automation overrides it, so a preset pins its sound 🧪
- [x] 000135 — **Silent-until-note default patch** — note-gated (`midi-in → adsr → vca.cv`
  with VCA `bias = 0`) so the instrument doesn't drone the instant it loads. The default
  patch is now `oscillator → svf → vca.in` for audio and `midi-in.gate → adsr → vca.cv` for
  control, with the VCA stored biased shut; tests assert silent at rest, audible on a note 🧪
- [x] 000136 — **Moog Minitaur** clone (mono bass): 2 saw/pulse oscillators + sub, ladder
  low-pass, glide. Shipped as `examples/minitaur.ron`: two detuned saws + a square sub an
  octave down (`pitch-osc`), a `glide` (portamento) stage on the pitch, an env-swept
  `ladder`, and a note-gated VCA. Added the `glide` node (control-signal slew over
  `glide_ms`, wrapping the `Smoother`)
- [x] 000137 — **Minimoog Model D** clone: 3 oscillators, ladder filter — the classic
  lead/bass voice. Shipped as `examples/minimoog.ron`: three detuned pitch-tracking
  oscillators (unison / +6 cents / octave-down) → mixer → env-swept `ladder` → note-gated
  VCA. Playable (tracks pitch), silent at rest. Added a `detune_semitones` param to
  `pitch-osc` for the oscillator stack
- [x] 000138 — **Moog Grandmother** clone (semi-modular): 2 oscillators, ladder filter, a
  spring-reverb-style voice. Shipped as `examples/grandmother.ron`: a saw + triangle
  (`pitch-osc`) → env-swept `ladder` → note-gated VCA → `reverb` (Freeverb, generous
  dry/wet for the spring ambience). Built from existing nodes (the `reverb` node stands in
  for the spring)
- [x] 000139 — **Behringer Neutron** clone: 2 oscillators, multimode filter, overdrive,
  delay in the voice. Shipped as `examples/neutron.ron`: two detuned `pitch-osc` → `add` →
  env-swept multimode `vcf` → `waveshaper` (overdrive) → note-gated VCA, with an in-voice
  `fb-delay` mixed back in (dry + wet) so echoes ring past the gate. Built from existing
  nodes
- [x] 000140 — **Korg Minilogue XD** clone: 2 VCO + a digital multi-engine voice, polyphony,
  digital effects. Shipped as `examples/minilogue.ron`: a **4-voice polyphonic** patch —
  four independent voice chains, each fed by a `voice-in` node bound to its own slot (the
  `PolyNoteSource`/`VoiceAllocator` the plugin already drives), summed into a `reverb`
  (digital effect). Each voice is two detuned saw VCOs + a triangle "digital" oscillator →
  env-swept `ladder` → note-gated VCA. (The digital engine is approximated with the
  triangle oscillator; a true wavetable oscillator is left as a future node.)
- [x] 000141 — **Drum-machine kit** patch(es): analog-style kick / snare / hat voices
  (TR-style), triggered per note. Shipped as `examples/drumkit.ron`: kick (pitch-enveloped
  `vco` sine), snare (`noise` + tone), hat (highpassed `noise`), each fired by a `key-gate`
  node on its own MIDI note (36 / 38 / 42). Added the `key-gate` node (+
  `PolyNoteSource::note_gate`) so drums read the polyphonic note source and play together
- [x] 000142 — **Multiband distortion / waveshaper** effect patch (fxpansion Maul-style):
  band-split → per-band drive/waveshape → mix. Shipped as `examples/maul.ron`: a rich saw
  split into three bands by two `svf` crossovers (low = lowpass, mid/high from the cascaded
  highpass), each band driven by its own `waveshaper` (foldback / saturate / hard-clip),
  then recombined. Built from existing nodes (the `svf` lowpass/highpass outputs are the
  crossover)
- [x] 000143 — 🧩 **Ladder-filter node** (`ladder`) — Moog-style four-pole (24 dB/oct)
  resonant low-pass, control-swept like the `vcf`. Zero-delay-feedback TPT design
  (accurately tuned, stable to self-oscillation). Shared prerequisite for 000136/000137;
  demonstrated by the env-swept `examples/ladder_bass.ron` factory patch 🧪
- [x] 000144 — 🧩 **MIDI-pitch-tracking oscillator** (`pitch-osc`) — a generator whose
  frequency follows the played note (`midi-in.pitch_hz → pitch-osc.pitch`), falling back to
  `base_hz` when no note is held. The enabler that makes 000136–000140 *playable* (plain
  `oscillator` is fixed-frequency; `vco` takes an exponential octave CV, not absolute Hz).
  Demonstrated by the playable `examples/mono_lead.ron` (tracks note pitch end-to-end) 🧪
- [x] 000145 — 🧩 **FM operator node** (`fm-op`) — a pitch-tracking sine carrier with a
  phase-modulation (`mod`) input and a harmonic `ratio`/`index`; chaining operators
  (`op → op.mod`) at musical ratios is the basis of FM synthesis (bell/EP/metallic tones) 🧪
- [x] 000146 — 🧩 **Supersaw oscillator node** (`supersaw`) — a pitch-tracking unison stack
  of up to nine PolyBLEP sawtooths, symmetrically detuned and amplitude-normalised, for fat
  trance/EDM leads 🧪
- [x] 000147 — 🧩 **AD (attack–decay) envelope node** (`ad`) — a one-shot envelope that fires
  a full attack→decay contour on each gate edge and ignores the release, for percussion and
  plucks (the second envelope generator alongside `adsr`) 🧪
- [x] 000148 — 🧩 **Envelope follower node** (`env-follower`) — rectifies an audio input and
  smooths it (attack/release ballistics) into a control signal, for auto-wah, dynamic
  tremolo and ducking (audio → CV) 🧪
- [x] 000149 — 🧩 **Sample & hold node** (`sample-hold`) — latches its `in` value on each
  rising edge of a `trig` input and holds it, for stepped-random modulation (noise + clock)
  and clocked LFO staircases 🧪
- [x] 000150 — 🧩 **Slew limiter node** (`slew`) — bounds a control signal's rate of change
  with separate rise/fall times, for portamento, rounded edges and lopsided ramps (a
  generalised, asymmetric `glide`) 🧪
- [x] 000151 — 🧩 **Ring modulator node** (`ring-mod`) — multiplies the input by an internal
  sine carrier (`freq_hz`, dry/wet `mix`), for metallic/clangorous inharmonic tones 🧪
- [x] 000152 — 🧩 **Bitcrusher node** (`bitcrusher`) — lo-fi degradation via bit-depth
  quantisation (`bits`) and sample-rate decimation (`downsample`), with dry/wet `mix` 🧪
- [x] 000153 — 🧩 **Subtract node** (`sub`) — `out = a - b`, the difference counterpart to
  `add` (phase cancellation, mid/side, error signals) 🧪
- [x] 000154 — 🧩 **Rectify node** (`rectify`) — `out = |in|`, full-wave rectifier (octave-up
  fuzz for audio, bipolar→unipolar for control) 🧪
- [x] 000155 — 🧩 **Attenuverter node** (`attenuvert`) — `out = in·scale + offset`, the
  workhorse CV utility (negative scale inverts; offset shifts range) 🧪
- [x] 000156 — 🧩 **Tremolo node** (`tremolo`) — periodic amplitude modulation by an internal
  sine LFO (`rate_hz`, `depth`), the vintage amp shimmer / pulsing pad 🧪
- [x] 000157 — 🧩 **Chorus node** (`chorus`) — a short LFO-modulated delay (`rate_hz`,
  `depth_ms`) mixed with the dry signal, thickening/widening via the moving-tap detune 🧪
- [x] 000158 — 🧩 **Pulse-width oscillator node** (`pwm`) — a pitch-tracking, PolyBLEP
  band-limited pulse with adjustable duty (`width`); 50% = square, narrower = brighter/nasal 🧪
- [x] 000159 — 🧩 **Hard-sync oscillator node** (`hard-sync`) — a slave saw hard-reset each
  master cycle (`sync_ratio`), the aggressive tearing "sync lead"; pitch-tracking 🧪
- [x] 000160 — 🧩 **Showcase example patches** for the new nodes — `examples/supersaw_lead.ron`
  (supersaw → ladder → note-gated VCA), `examples/fm_ep.ron` (2-operator FM electric piano),
  `examples/auto_wah.ron` (saw drone → tremolo → envelope-follower-swept VCF); each wired into
  the `vb-plugin` examples manifest with a targeted engine test 🧪
- [x] 000161 — 🧩 **AudioUnit validation gate** — `cargo xtask validate-au` reads the built
  `.component`'s codes and runs Apple's `auval` (authoritative AU check: format negotiation,
  render at all sample rates, params, MIDI); a non-blocking CI `au validate` job builds the
  AUv2 and runs it, so the VST3/AU export is proven, not just the CLAP (`auval_args` unit-tested) 🧪
- [x] 000166 — **Behringer Edge** clone (analog percussion): a mono percussion voice — the
  second Behringer requested alongside Neutron (000139). Shipped as `examples/edge.ron`: a
  pitch-tracking tone + `noise` → resonant `ladder` → note-gated `vca`, the amp driven by a
  one-shot `ad` envelope so a note fires a short **hit that decays** (kick/tom/zap), unlike a
  sustained lead/bass. Built from existing nodes; wired into the examples manifest with a
  `edge_is_a_percussive_hit_that_decays` test ([#208](https://github.com/ice1x/VST_bomb/issues/208)) 🧪
- [x] 000168 — **Roland TB-303** clone (acid bass): the classic squelchy mono-bass. Shipped as
  `examples/tb303.ron` and offered in the editor's factory-patch list: a single saw (`pitch-osc`)
  with `glide` (the 303 **slide**) → a **high-resonance** `ladder` swept by a filter `ad`
  env-mod (the "wow"/squelch) → note-gated `vca` with its own short amp `ad` (a plucky, decaying
  note). Built from existing nodes; a `tb303_is_silent_at_rest_squelches_on_a_note_and_decays`
  test asserts the silent→squelch→decay behaviour ([#219](https://github.com/ice1x/VST_bomb/issues/219)) 🧪
- [x] 000169 — **Roland SH-101** clone (mono synth): the versatile saw/pulse/sub mono-synth.
  Shipped as `examples/sh101.ron` and in the factory-patch list: saw (`pitch-osc`) + pulse
  (`pwm`) + square sub + a hint of `noise`, leveled (`attenuvert`) and summed → resonant
  `ladder` → note-gated `vca`, with **one ADSR fanned out** to both the filter cutoff and the
  amp (the SH-101 has a single envelope) and `glide` for portamento. Built from existing nodes;
  a `sh101_is_silent_at_rest_and_sustains_a_note` test asserts silent→sustained (distinct from
  the plucky 303/Edge) ([#221](https://github.com/ice1x/VST_bomb/issues/221)) 🧪
- [x] 000170 — **Roland Juno-60** clone (polysynth): the lush chorus polysynth. Shipped as
  `examples/juno60.ron` and in the factory-patch list: a **4-voice polyphonic** pad (each
  `voice-in` voice a saw + pulse (`pwm`) + square sub DCO → resonant `ladder` → note-gated
  `vca`, one `adsr` to filter+amp), the four voices summed into the signature **`chorus`**.
  Uses the same `PolyNoteSource`/voice-allocator the plugin drives (as the Minilogue clone
  does). A `juno60_is_silent_at_rest_and_polyphonic_notes_sound_through_the_chorus` test plays
  a chord into the poly source ([#223](https://github.com/ice1x/VST_bomb/issues/223)) 🧪
- [x] 000171 — 🧩 **Wavetable oscillator node** (`wavetable`) — a pitch-tracking oscillator that
  **morphs** across a built-in four-frame bank (sine → triangle → saw → square, each synthesised
  additively so it is band-limited by construction). The `position` (a parameter **summed** with
  a `position` CV input, so an envelope/LFO can sweep the timbre) blends between frames with
  linear phase+frame interpolation. Closes the "future wavetable oscillator" gap noted for the
  Minilogue's digital engine; unit-tested for audible/finite output, pitch tracking, and that
  higher `position` (and CV) adds harmonics ([#225](https://github.com/ice1x/VST_bomb/issues/225)) 🧪
- [x] 000172 — 🧩 **Karplus–Strong plucked-string node** (`karplus`) — a **gated** physical-model
  oscillator: each rising `gate` edge fills a delay line of length `sample_rate / pitch` with a
  noise burst (the "pluck"), then feeds it back through a two-point averaging low-pass so the
  timbre darkens and rings down like a real string. `pitch` is latched at pluck time (a string
  does not change length while ringing), `damping` sets the sustain, and `tone` blends between the
  full averaging low-pass (dull) and a bright string. First plucked/percussive voice in the
  library — a new sound class, not a subtractive-synth clone. Unit-tested for silence-until-pluck,
  ring-down decay, retrigger, reset, and pitch tracking recovered by autocorrelation 🧪
- [x] 000173 — 🧩 **Formant (vowel) filter node** (`formant`) — shapes any input into a **vocal**
  timbre with three parallel RBJ band-pass "formants" (F1/F2/F3) summed. The `vowel` parameter
  (summed with a `vowel` CV input, so an envelope/LFO sweeps the mouth shape) **morphs** across
  the five cardinal vowels **a → e → i → o → u** by interpolating the classic sung-vowel formant
  table; `resonance` scales the formants' Q. Makes a buzzy source (saw/pulse/noise) "speak" —
  another new tone class. Unit-tested for finite passthrough, band-pass resonance on F1,
  vowel-dependent frequency response, vowel-CV summing, and reset 🧪
- [x] 000174 — 🧩 **Modal resonator node** (`resonator`) — the "resonator" half of physical
  modelling (complement to the `karplus` exciter): a bank of five high-Q band-pass **modes**
  tuned to partials of a base pitch and summed, so any excitation (impulse, noise burst, drum
  hit, full signal) rings into a pitched, metallic tone. `decay` maps to the modes' Q (ring
  length); `structure` morphs the partials from a **harmonic** series toward an **inharmonic**
  bell/tubular set (1, 2.76, 5.40…); pitch tracks a `pitch` control input. A `q` makeup gain
  keeps the ring audible and its amplitude decay-independent (a high-Q band-pass has a tiny
  transient gain). Unit-tested for ring-after-strike, decay-controls-length, pitch tracking
  (autocorrelation), harmonic-vs-inharmonic difference, and reset 🧪
- [x] 000175 — 🧩 **Granular processor node** (`granular`) — a "grain cloud": continuously
  records the audio input into a 2 s rolling buffer and replays it as a swarm of short,
  overlapping, Hann-windowed **grains** read from a chosen point behind the write head. Because
  playback position is decoupled from the record head it **time-stretches** (`position`, summed
  with a CV input, scrubs the recent past) and **pitch-shifts** (`pitch` — grain playback speed)
  independently; `grain_ms`, `density` (grains/s) and `spray` (position randomisation, seeded
  RNG for deterministic tests) shape the texture from smooth pad to glitchy cloud. A new synthesis
  class. Unit-tested for finite/audible output, silence-without-input, density-adds-energy,
  pitch-brightens-the-output, and reset 🧪
- [x] 000177 — 🧩 **End-to-end interface tests** (`vb-editor/tests/e2e_interface.rs`) — headless
  journeys driving `EditorApp` through the **same public API the plugin/standalone UI calls**:
  the launch seed, camera hit-testing (click-to-select, drag-to-wire), the inspector's parameter
  edits, and the project RON export/import buttons. Covers the launch invariant (the canvas must
  open on the real **gated** default voice, not a bare oscillator) and a full session
  (add → wire → edit → export → re-import → select). **Caught a real bug on the first run:**
  `PatchView::to_doc` built the document from the graph alone and dropped every edited parameter
  value, so exporting/saving a project silently lost all knob edits — now `to_doc` copies the
  inspector values into the document (regression-guarded by
  `an_edited_parameter_survives_a_project_round_trip`) 🧪
- [x] 000178 — 🧩 **Resizable plugin window** (⚠️ **reverted in 000180**) — wrapped the canvas in
  `nih_plug_egui`'s `ResizableWindow` for a drag-to-resize corner. It broke on the AU/clap-wrapper
  path (resizing left most of the enlarged window black — the host grew the window but egui's
  surface did not track it), so it was reverted. The harmless part — splitting `render` into
  `render_chrome` + `canvas_ui` — was kept.
- [x] 000179 — 🧩 **Fixed control-rate quantum (bounce == realtime)** — the plugin processed the
  host's whole buffer as one engine pass, so block-rate modulation (the `ladder` cutoff is
  retuned once per pass; note gates land at block start) resolved at the host's buffer size.
  A Logic **offline bounce** uses a larger buffer than realtime playback, so the filter's attack
  sweep stepped coarsely and the low-frequency attack thickened — audibly different from
  realtime. `process` now splits the host block into fixed **64-sample quanta**
  (`CONTROL_QUANTUM`) and runs the engine per quantum (`process_in_quanta` + `deliver_to_host_at`
  writing each quantum at its offset), so modulation resolves identically regardless of the host
  buffer. Tested: quanta equal a single pass when the block fits one quantum, and a 1024-sample
  block quantised differs from one big pass in the filter attack 🧪
- [x] 000180 — 🧩 **Revert the resizable plugin window (000178)** — the `ResizableWindow` drag
  corner left most of the window black after a resize on the AU/clap-wrapper path (the host grew
  the window but egui's surface did not track the new size). Reverted to the fixed-size window,
  which renders correctly. The `render` split (`render_chrome` + `canvas_ui`) is kept, and the
  render smoke test is generalised to render the editor at a small **and** a large context size
  (`render_fills_the_context_at_any_size`) — the coverable part; the host↔window surface layer
  where the resize bug lived has no headless test surface. Real host-driven resize waits on
  nih-plug implementing it upstream (`ext_gui_can_resize` still returns `false`) 🧪

### Epic 15 — Panel view (interface constructor)

A **performance-facing control surface** distinct from the node graph — Reaktor's Panel vs
Structure. The graph is where you *build* the instrument; the Panel is what a player *tweaks*.
Built up in slices: an auto-generated panel first, then custom widget placement/binding, then
per-widget styling and a saved layout in the project.

- [x] 000181 — 🧩 **Panel view toggle + auto-panel** — a `ViewMode { Structure, Panel }` with a
  **Blocks / Panel** switch in the toolbar. Blocks is the existing node graph; Panel is a new
  surface that auto-generates one live-bound slider per node parameter (labelled `"<kind>
  <param>"`), each editing the same value through `PatchView::set_param` that the inspector and
  the engine use. `panel_widgets()` returns the binding set as data so it is testable without an
  egui frame. Both the standalone and the plugin get the toggle (they share `render`). Tested:
  starts on Blocks and toggles; the panel lists every node parameter; a panel edit updates the
  shared value; both surfaces render at a small and large size. *(Next: custom widget
  placement/binding, then styling + a saved panel layout in the project.)* 🧪
- [x] 000182 — 🧩 **Grouped panel modules with knobs** — the flat wall of look-alike sliders
  becomes a **rack of titled modules**: `panel_modules()` groups the bindings by node (one framed
  block per param-bearing node) and gives each block a unique title, numbering repeated kinds
  (`pitch-osc 1/2/3`) while leaving a once-only kind bare (`ladder`). Each module renders its
  parameters as **rotary knobs** (reusing the inspector's `knob_widget`, live-bound through
  `set_param`) in a wrapping grid, so the Panel reads like a synth front panel. Model is data —
  tested for grouping/coverage, ordinal titling, and the empty-patch case; both surfaces still
  render at any size. *(Next: custom widget placement/binding, then styling + a saved panel
  layout in the project.)* 🧪
- [x] 000183 — 🧩 **Per-control widget type on the panel** — each panel control is now drawn as
  the [`WidgetKind`] its parameter is assigned (knob / slider / switch), and **right-clicking a
  control opens a menu to change that type** — the first "constructor" gesture on the surface.
  `PanelBinding` carries the `widget`, populated from the slot and honored by both the flat and
  grouped views, so the panel and the inspector stay in lock-step (choosing a control in either
  is reflected in both). Type and value edits go through the same `set_widget` / `set_param`
  paths the inspector uses. Tested: a binding reports its assigned type and tracks a re-assign
  across both views. *(Next: custom widget placement/drag, then styling + a saved panel layout in
  the project.)* 🧪
- [x] 000184 — 🧩 **Draggable panel layout** — panel modules are now **free-positioned**: grab a
  module's **title bar** to drag it anywhere on the surface. Until placed, a module auto-flows into
  a deterministic grid slot (`PANEL_COLS` columns), so a fresh patch still reads cleanly.
  `PatchView::panel_position` / `set_panel_position` hold the placement (view-only — like a widget
  assignment it never touches the audio revision), and `EditorApp::panel_layout` resolves each
  module to its explicit spot or its auto-grid slot. Tested: the auto-grid order, that an explicit
  placement wins, and that moving a module is not audible. *(Next: per-widget styling, then a saved
  panel layout persisted in the project.)* 🧪
- [x] 000185 — 🧩 **Panel layout persisted in the project** — the arrangement now travels with the
  patch. `NodeEntry` gained two optional, presentational fields — `panel_pos` (the module's dragged
  position) and `widgets` (per-parameter control-kind labels) — both `#[serde(default,
  skip_serializing_if)]`, so a patch nobody arranged emits nothing new and older documents still
  load (format stays version 1, forward- and backward-compatible). `PatchView::to_doc` writes them,
  `apply_doc` restores them (positions into `panel_positions`, labels back into each slot's
  `WidgetKind` via `WidgetKind::from_label`), so saving and reopening a project brings the panel
  back exactly. Tested: a placed + retyped panel round-trips through the RON in both `vb-project`
  (field round-trip / omitted-when-default) and `vb-editor` (end-to-end export → import). *(Follow-up:
  capture live view-only panel edits into the plugin's DAW-saved state; then per-widget styling.)* 🧪
- [x] 000186 — 🧩 **Plugin saves in-window panel edits into the DAW project** — the plugin used to
  serialize its host state from a `patch_ron` frozen at load, so *any* in-window edit (and a
  rearranged panel in particular) was lost when the DAW project was reopened. The editor now bumps
  a separate `PatchView::view_revision` on presentational edits (panel placement, control type),
  and the plugin's editor mirror re-serializes the live document into the host state
  (`VstBombParams::set_patch_ron`) on **both** audible and view-only changes — rebuilding the audio
  engine only for the audible ones. So a project saved after arranging the panel in the DAW reopens
  with that arrangement. Tested: `view_revision` bumps on placement/retype but not on audible edits
  (and vice-versa), and `set_patch_ron` makes the saved host state carry the edited patch. *(Next:
  per-widget styling.)* 🧪
- [x] 000187 — 🧩 **Module accent colours (panel styling)** — a module can be given an **accent
  colour** (right-click its title → a small preset palette, or "default"); the accent frames the
  module and tints its title, so a panel can be laid out and colour-coded like a real instrument
  front. The colour is view-only (`PatchView::set_panel_color`, bumps `view_revision`) and **saved
  with the patch**: `NodeEntry` gained an optional `panel_color` (`#rrggbb`) field, written by
  `to_doc` and restored by `apply_doc` (a malformed hex falls back to the theme). Tested: hex ↔
  colour round-trip, `view_revision` bumps on recolour but stays inaudible, the field round-trips
  through the project RON (and is omitted when unset), and the end-to-end editor reopen restores
  the accent. *(Next: per-control size and custom labels.)* 🧪
- [x] 000189 — 🧩 **Vertical faders (and faders by default for envelopes)** — added a `Fader`
  widget kind — a vertical fader — rendered by both the panel and the inspector, and offered in the
  right-click control-type menu. Envelope segments now **default** to a fader (`default_widget`
  returns `Fader` for a `*_s` time or `sustain` level), so an ADSR reads as the attack/decay/sustain/
  release fader stack players expect instead of a row of identical dials. Like any control type the
  choice persists with the patch (the `fader` label round-trips through the saved layout). Tested:
  envelope params default to a fader while other params stay knobs, and the label round-trips. *(Next:
  per-control size and custom labels.)* 🧪

### Epic 14 — Minitaur as the default synth

Make the plugin/standalone load a recognisable **Moog Minitaur mono-bass** as its default
sound (instead of the generic gated osc→filter→vca voice), using the clone as the vehicle to
finish the DSP modules and UI it exercises. Minitaur is the **simplest** Epic-13 clone (mono,
2 osc + sub, one ladder filter, envelopes, LFO, glide), so it is the right default and a good
shake-out. Most building blocks exist (incl. `attenuvert` for CV amount), so this is largely
assembly + a default-patch swap + UI polish, not new DSP. Tracked as issue
[#203](https://github.com/ice1x/VST_bomb/issues/203).

- [x] 000162 — **Minitaur voice spec & fidelity audit** — [docs/synths/minitaur.md](docs/synths/minitaur.md):
  target voice vs. the current `examples/minitaur.ron` (one shared envelope for filter+amp, no
  LFO, bare `add` mixer) with the gap list and the target graph. No new DSP nodes needed —
  `attenuvert` already covers the filter-EG amount, LFO depth, and per-osc level. Baseline
  locked by a `minitaur_is_silent_at_rest_and_sings_on_a_note` test ([#204](https://github.com/ice1x/VST_bomb/issues/204)) 🧪
- [x] 000163 — **Faithful Minitaur voice** — `vb_engine::minitaur_patch()` (from which
  `examples/minitaur.ron` is generated, kept in sync by an anti-drift test): **separate filter
  and amp envelopes**, a **leveled oscillator mix** (per-osc `attenuvert`), glide, pinned
  values; tests assert silent at rest, audible + bass on a note, and pitch tracking. The LFO is
  split out to 000167 — routing it to cutoff needs a control-rate mixer (`add`/`attenuvert` are
  audio-typed, and `Audio` is not compatible with `Control`) ([#205](https://github.com/ice1x/VST_bomb/issues/205)) 🧪
- [x] 000167 — 🧩 **Control-rate math nodes** (`cv-add`, `cv-attenuvert`) + wire the **Minitaur
  LFO** — the audio `add`/`attenuvert` can't carry CV (`Audio` ≠ `Control`), so added their
  `Control`-typed twins (same processors, control ports) and routed
  `lfo → cv-attenuvert(depth) → cv-add ← filter-EG → ladder.cutoff` in `minitaur_patch`; LFO
  depth 0 by default (clean bass, wobble on demand). Closes gap 2 from
  [docs/synths/minitaur.md](docs/synths/minitaur.md) ([#211](https://github.com/ice1x/VST_bomb/issues/211)) 🧪
- [x] 000164 — **Make Minitaur the default patch** — `vb_engine::default_patch` delegates to
  `minitaur_patch`, so the plugin/DAW ships the Minitaur bass; the editor's starting patch
  loads the same voice from the committed `examples/minitaur.ron` (auto-laid-out), so the
  standalone opens on it too — both silent-until-note. Fixed a real bug found here:
  `PatchView::apply_doc` was resetting every knob to its registry default on load (a preset's
  VCA `bias 0` sprang open and droned) — it now carries stored per-node values. Engine/
  standalone/editor tests updated ([#206](https://github.com/ice1x/VST_bomb/issues/206)) 🧪
- [x] 000165 — **UI polish for factory patches** — a **"factory patches"** palette section
  (`crate::factory`: the instrument clones — Minitaur, Model D, Grandmother, Neutron, Edge,
  Minilogue, drum kit — each `include_str!`'d from `examples/`) with a one-click
  `EditorApp::load_factory_patch` that replaces the canvas; an anti-rot test parses every
  entry. Palette **node-category grouping** was already present (headers per `category`); the
  inspector already labels each parameter by name. Richer per-unit inspector annotations are a
  minor visual follow-up left to a human pass ([#207](https://github.com/ice1x/VST_bomb/issues/207)) 🧪
- [x] 000202 — **Minitaur LFO → pitch destination** — the hardware LFO has **two** independent
  destinations, each with its own amount knob (VCO LFO AMOUNT → pitch, VCF LFO AMOUNT → cutoff;
  per Moog's Minitaur manual), but `minitaur_patch` only wired the cutoff leg (000167). Fanned
  the one LFO out to a second `cv-attenuvert(pitch-depth) → cv-add ← glide → each VCO.pitch`
  (control-rate, since the glided pitch is a `Control` signal); **pitch depth 0 by default** so
  the shipped bass is unchanged, but the panel now has a working vibrato knob. Brings the code
  up to its own spec in [docs/synths/minitaur.md](docs/synths/minitaur.md) 🧪

**Maintaining the list:** mark `[x]` when done, do not reuse numbers; add new tasks at
the end with a new global number (000182, …); split a large task into 🧩 subtasks with
their own numbers.

## 📄 License

Apache-2.0 — see [`LICENSE`](LICENSE).
