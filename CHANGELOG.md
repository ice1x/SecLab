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
- **AI assistant (`vb-ai`)** — a grounded, offline-testable assistant stack: system prompt
  built from the live node reference and the RON output contract; a provider abstraction
  with a BYOK keystore and a privacy gate (offline switch, consent, a "what will be sent?"
  payload inspector); a token/cost budget guard; propose → validate → repair convergence;
  and generate / edit / explain / suggest / repair / critique skills.
- **Documentation** — a [node reference](docs/nodes.md), a
  [user guide](docs/first-plugin.md) (build your first plugin), a
  [developer guide](docs/adding-a-node.md) (add your own node), and tested starter
  patches under [`examples/`](examples).

[Unreleased]: https://github.com/ice1x/VST_bomb/commits/main
