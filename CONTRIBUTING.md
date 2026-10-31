# Contributing to SecLab

Thanks for helping build a no-code/low-code VST/AU plugin editor. This document is the
short version of how we work. When in doubt, match the surrounding code and keep changes
small and reviewable.

## Golden rules

- **TDD, always.** Write a failing test first, make it pass with the minimum code, then
  refactor. No functionality lands without tests — features, bug fixes and refactors all
  ship with tests. See [Testing](#testing).
- **SOLID.** The engine, nodes, UI and export are separated by clear interfaces (traits).
  Nodes plug into the engine as extensions; keep dependencies pointing inward toward
  `seclab-core`. Don't reach across layers.
- **Green before push.** `fmt`, `clippy -D warnings` and `test` must pass locally before
  every commit, and in CI before merge.

## Workspace layout

```
crates/
  seclab-core/        graph model, port types, scheduler, sample processing
  seclab-nodes/       DSP node library
  seclab-project/     project serialization (RON)
  seclab-runtime/     real-time graph execution
  seclab-editor/      egui node editor
  seclab-standalone/  standalone app (binary)
  seclab-plugin/      CLAP export / VST3-AU packaging
```

## Local checks

Run all three before committing (this is exactly what CI runs):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo fmt --all` (without `--check`) applies the formatting. Clippy is denied at
warning level both in CI and in-tree via `[workspace.lints]`, so a warning is an error.

## Testing

- **Unit tests** live next to the code (`#[cfg(test)] mod tests { … }`), written first.
- **Integration tests** go in a crate's `tests/` directory and exercise realistic
  workflows once a feature works.
- Real-time code (`seclab-runtime`) must be tested for **no allocations/locks** on the audio
  path where practical.

## Branches & commits

- Never commit or push directly to `main`. Branch from an up-to-date `main`.
- Branch names: `feature/<short-description>` or `fix/<short-description>`.
- Push the feature branch and open a PR against `main`.
- Commit messages have a short imperative **title**, a blank line, then a **body** that
  explains what changed and why (which files/modules and the reasoning). Reference backlog
  item numbers (e.g. `000011`) where relevant.

## Backlog

The task backlog lives in [`README.md`](README.md) as an incremental checkbox list
(`000001…`), mirrored as GitHub issues labelled `epic`. Mark items `[x]` in the same PR
that completes them. Add new tasks at the end with a new global number.
