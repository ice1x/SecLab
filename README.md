# SecLab

**Cross-platform no-code / low-code editor for VST/AU plugins.**

Wire DSP nodes together in a visual editor — oscillators, filters, envelopes, math,
MIDI/audio I/O — hear the result in real time, and export a ready-to-use **VST3 / AU /
CLAP** plugin plus a standalone app. No coding required. Written in **Rust**; the UI is
[egui](https://github.com/emilk/egui) (compiles to WASM for a future WebUI). A built-in
**AI assistant** turns a plain-language description into a validated patch graph you can
audition and export.

## Quick start

```bash
# run the standalone editor
cargo run -p seclab-standalone

# build the VST3 / AU / CLAP bundles
cargo xtask bundle seclab-plugin

# tests / lint / format (required before every commit)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Faster test runs: [`cargo nextest run`](https://nexte.st/) (config in
[`.config/nextest.toml`](.config/nextest.toml)); doctests via `cargo test --doc`.

## Documentation

- [Overview & design](docs/overview.md) — goals, stack & key decisions, the AI-assistant design.
- [Roadmap & backlog](docs/backlog.md) — epics and the full task checklist.
- [Build your first plugin](docs/first-plugin.md) — oscillator → filter → envelope → output.
- [Adding your own node](docs/adding-a-node.md) — extend the DSP library.
- [Node reference](docs/nodes.md) — every built-in node, its ports and parameters (auto-generated).
- [CHANGELOG](CHANGELOG.md) — notable changes and the versioning policy.
- [`examples/`](examples) — small, tested starter patches.

## License

**PolyForm Strict License 1.0.0** — source-available, not open-source: you may read the
code, but using, modifying, redistributing or commercializing it is not permitted. See
[`LICENSE`](LICENSE) for the full terms and [`NOTICE`](NOTICE) for third-party components
(which remain under their own, permissive licenses).
