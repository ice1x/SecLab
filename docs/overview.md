# SecLab — overview & design

Project goals, the stack and key decisions, principles, and the AI-assistant design.
For the task checklist see [backlog.md](backlog.md); for changes see [CHANGELOG](../CHANGELOG.md).

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
SecLab/
├─ crates/
│  ├─ seclab-core/        # graph model, port types, scheduler, sample processing
│  ├─ seclab-nodes/       # DSP node library (osc, filters, envelopes, math, MIDI)
│  ├─ seclab-project/     # project serialization (graph format, save/load, versions)
│  ├─ seclab-runtime/     # real-time graph execution (buffers, parameters, MIDI)
│  ├─ seclab-editor/      # egui node editor (canvas, wires, inspector, undo)
│  ├─ seclab-standalone/  # standalone app (cpal + editor)
│  └─ seclab-plugin/      # CLAP export (nih-plug), VST3/AU build via clap-wrapper
├─ README.md          # short overview (full design: docs/overview.md, plan: docs/backlog.md)
└─ LICENSE            # PolyForm Strict 1.0.0
```

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
                                                    validate (seclab-core) ──▶ audition
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
   same RON `ProjectDoc` the editor loads, so `seclab-core` validation is the hard gate —
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

Detailed checklist — in [backlog.md](backlog.md).
