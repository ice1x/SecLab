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

- **Shared STFT overlap-add engine extracted to `vb_nodes::stft` (000220).** The `pitch-shifter`
  and `spectral-freeze` nodes each inlined the same short-time-Fourier scaffolding — a Hann-windowed
  analysis FIFO, the per-hop forward FFT, the inverse FFT with constant-overlap-add resynthesis, the
  output FIFO/accumulator shifts and a latency-matched dry delay — differing only in the *spectral
  transform* in the middle. That machinery now lives once in an `Stft` engine (`dry_delay`, `push`,
  `forward`, `synthesize`, `reset`, plus the shared `FFT_SIZE`/`OSAMP`/`STEP_SIZE`/`HALF`/`LATENCY`
  constants and the Hann² COLA normalisation); each node keeps only its `transform` over
  `self.stft.fft_work` and its own per-bin state. Mirrors the biquad-core extraction of 000215. Pure
  internal refactor — no change to either node's public API, registry descriptor, parameters or
  `docs/nodes.md`, and every existing per-node DSP-law test passes unchanged; the engine adds its own
  tests (an identity round-trip reconstructs a tone at unit gain). Future spectral nodes now cost
  only their transform.
- **Shared Direct Form I biquad core extracted to `vb_nodes::biquad` (000215).** The four RBJ
  cookbook nodes — `filter` (`Biquad`, LP/HP/BP/notch), `shelf-eq`, `peak-eq` and `allpass` — had
  each inlined an identical normalised-coefficient struct, the two-in/two-out delay state, the
  per-sample difference equation and `reset`. That arithmetic now lives once as
  `BiquadCoeffs::normalized` (the single `/a0`) and `BiquadState::step`/`reset`; every node keeps
  only its own cookbook coefficient formula and defers the running to the shared state. Purely
  internal: no change to any node's public API, registry descriptor, parameters or `docs/nodes.md`,
  so all existing per-node DSP-law tests pass unchanged. The core carries its own unit tests,
  including a bit-for-bit (`to_bits`) check that `step` equals a hand-rolled Direct Form I
  recurrence.

### Fixed

- **Library re-port: enforce the unique patch title in `vb-library`, not drevo (000208).** drevo
  dropped intrinsic title uniqueness (drevo #595: `create_node` now accepts a repeated title), so
  the `library / drevo` CI job went red on `a_duplicate_title_is_rejected`. The library owns the
  "title is the key" invariant, so `import_patch` now checks `get_node_by_title` before any write
  and refuses a taken title with `DrevoError::DuplicateTitle` (the error variant still exists;
  only the automatic enforcement was removed). Adapted to the actual drevo API rather than pinning
  an old drevo checkout. The non-required `library / drevo` job is green again.

### Added

- **Whisperize node (`whisperize`, 000223).** The phase-vocoder "whisper voice" — the complement of
  `robotize`. Where robotization replaces every bin's phase with a *fixed* value (zero) to impose a
  single pitch, whisperization replaces each bin's phase with a *random* value while keeping its
  magnitude `|X(k)|`. Destroying the phase coherence (between bins and between frames) removes all
  periodic/pitched structure, but because the magnitude spectrum is preserved the formant / timbre
  envelope survives: a sung vowel comes out as the same vowel *whispered* — the resonances are there,
  the pitch is gone. The DAFX (Zölzer) whisperization formula `ft = abs(f)·exp(i·random_phase)`. The
  random phase comes from a seeded xorshift32 PRNG (the same generator and seed as `noise`), so a
  patch renders identically every time and `reset` replays from the seed — determinism is part of
  the node contract. Third node built on the shared `vb_nodes::stft` engine (000220), memoryless per
  frame apart from the PRNG state. Single `mix` parameter; `effect` category; reports
  `latency_frames`; `set_params` updates live. A processing utility, so no factory patch. Unit-tested
  on the DSP law — a clean 440 Hz tone has tonality (coherent amplitude² / power) ≈1 while the
  whispered output drops to ≈0.0005 (coherence destroyed) with its energy still in the source band
  (≫ the 5 kHz floor); plus determinism/reset-replay, silence-in/out, verbatim dry mix, bounded
  output and `set_params` clamping; a registry behaviour test confirms the tonal collapse.
  `docs/nodes.md` regenerated 🧪
- **Robotize node (`robotize`, 000222).** The classic phase-vocoder "robot voice": each short-time
  frame keeps its magnitude spectrum `|X(k)|` and discards the measured phase, so every resynthesised
  grain is identical in phase and the overlap-add becomes a train of identical pulses spaced one hop
  apart. That imposes a constant period of `STEP_SIZE` samples on the output — the pitch collapses to
  `sample_rate / STEP_SIZE` (≈187.5 Hz at 48 kHz) and its harmonics **regardless of the input pitch**,
  while the magnitude spectrum (timbre/formant envelope) is preserved: a monotone, buzzy robot version
  of the source. Follows the DAFX (Zölzer) robotization formula (`r = abs(f)`); the reference
  `fftshift` that centres the zero-phase pulse under the Hann taper is folded into the frequency
  domain as the `(-1)^k` half-frame shift, since the shared engine applies no shift. Second node built
  on the shared `vb_nodes::stft` engine (000220) and memoryless per frame like `spectral-gate` — one
  short per-frame transform, no phase accumulation. Single `mix` parameter; `effect` category; reports
  `latency_frames`; `set_params` updates live. A processing utility, so no factory patch. Unit-tested
  on the DSP law — the robotised output's autocorrelation at lag `STEP_SIZE` is ≈1 (periodic at the
  hop) while the dry tone is not, and a different input pitch robotises to the same hop period — plus
  silence-in/silence-out, verbatim dry mix, audible-and-bounded level and `set_params` clamping; a
  registry behaviour test confirms the fixed hop period. `docs/nodes.md` regenerated 🧪
- **Spectral gate node (`spectral-gate`, 000221).** A frequency-domain noise gate: where a
  time-domain gate opens or closes on the whole signal's level, this makes the decision **per
  frequency bin**. Each short-time frame it finds the loudest bin and keeps only the bins within
  `threshold_db` of that peak, zeroing the rest — so a steady tone over a hiss floor comes out clean
  (the tone's bins survive, the broadband hiss between partials is gated), and pushed harder it
  strips a sound to its strongest partials. The threshold is relative to the per-frame peak, so it
  tracks level automatically. First node built on the shared `vb_nodes::stft` engine extracted in
  000220 — the whole node is one ~15-line per-frame transform (keep the surviving bins' analysed
  complex value, zero the rest), no phase state. Parameters `threshold_db` (−120..0) and `mix`;
  `effect` category; reports `latency_frames`; `set_params` updates live. A processing utility, so
  no factory patch. Unit-tested: a near-open gate reconstructs the input, a −40 dB partial is gated
  at a −30 dB threshold and returns at −50 dB, silence stays silent, a dry mix is verbatim, output
  bounded; a registry behaviour test confirms the partial is gated. `docs/nodes.md` regenerated 🧪
- **Spectral freeze node (`spectral-freeze`, 000219).** An STFT effect that captures the current
  short-time spectrum and sustains it indefinitely — an instant of sound becomes an endless,
  shimmering pad. While its `freeze` control input is low it resynthesises the input faithfully (a
  near-passthrough); while `freeze` is high it holds the last analysed magnitude spectrum and keeps
  advancing each bin's phase by its captured per-hop increment, so the drone keeps ringing even if
  the input falls silent, and ignores later input until released. Reuses the same STFT overlap-add
  engine as the pitch shifter (Hann-windowed frames, forward FFT, per-bin phase tracking, inverse
  FFT, overlap-add) on the shared `vb_nodes::fft` radix-2 transform: each hop the magnitude and raw
  inter-hop phase advance are captured into the held spectrum unless frozen, and the synthesis phase
  always accumulates the held advance (so unfrozen it telescopes back to the true phase = faithful
  reconstruction, frozen it rotates the held magnitudes at their captured pitch). Input 0 audio,
  input 1 `freeze` control (≥0.5 freezes), parameter `mix` (dry → resynthesised); `effect` category;
  reports `latency_frames`; `set_params` updates live. Unit-tested for faithful unfrozen
  reconstruction, a frozen tone sustaining through silence, a freeze ignoring later input, a
  verbatim dry mix, silence, and bounded output; a registry behaviour test drives the freeze
  control. `docs/nodes.md` regenerated 🧪
- **Channel vocoder node (`vocoder`, 000218).** The classic "robot voice": a two-input effect that
  imprints the moment-to-moment spectral envelope of a **modulator** (input 1, typically a voice)
  onto a **carrier** (input 0, typically a synth). The modulator is split into a bank of band-pass
  filters, each band's level is tracked by an envelope follower, and the carrier is split by the
  **same** bank with each carrier band multiplied by its modulator band's envelope before the bands
  are summed — so a sustained synth "speaks" the modulator. 16 log-spaced constant-Q bands from
  80 Hz to 7 kHz (the band count follows the Moog 16-channel vocoder; log spacing matches pitch
  perception, as in the MFOS design), each an RBJ band-pass biquad run on the shared
  `vb_nodes::biquad` core; the per-band Q is derived from the band spacing so the bands tile. Reuses
  the Direct Form I core extracted in 000215. Parameters `release_ms` (envelope follower smoothing,
  1..500) and `mix` (dry carrier → vocoded); `effect` category; `set_params` updates live. Like the
  EQ/dynamics nodes it is a processing utility (two audio inputs), so it ships without a factory
  "instrument" patch. Unit-tested for the vocoder law (a flat carrier takes the modulator's band;
  moving the modulator band moves the output band), silence gating from either input, a verbatim
  dry mix, log-spaced centres, and bounded output; a registry behaviour test drives it with two
  inputs. Primary sources: Wikipedia "Vocoder", Moog 16-channel (50–5080 Hz), MFOS 12-channel band
  table. `docs/nodes.md` regenerated 🧪
- **Showcase patch for the pitch shifter — Octave Stack (000217).** The new `pitch-shifter` now
  ships as a note-gated factory instrument so it is playable from the palette, not just wireable:
  **Octave Stack** (`examples/octave_stack.ron`) is a 220 Hz saw run through `pitch-shifter` at
  +12 semitones with a 50 % dry/wet `mix`, so the fundamental and its octave sound together for a
  thick, bright lead — the shifter's own `mix` gives the layered octave in a single node (no
  splitter/mixer), and its latency-matched dry path keeps the two layers time-aligned. In the
  example manifest + factory palette with a silent-at-rest / audible-on-note behavioural test (the
  on-note window is widened to clear the vocoder's STFT latency).
- **Phase-vocoder pitch shifter node (`pitch-shifter`, 000216).** Transposes the input up or down
  **without** changing its speed — the complement to `freq-shifter` (which adds a constant Hz to
  every partial and so goes inharmonic): a pitch shifter *multiplies* each partial's frequency by a
  ratio, so harmonics stay integer multiples and the tone keeps its character an octave away. The
  method is Stephan M. Bernsee's canonical STFT `smbPitchShift`: overlapping Hann-windowed frames
  are FFT'd, each bin's *true* frequency is recovered from the inter-frame phase advance, the
  magnitudes and true frequencies are moved to bins scaled by the pitch ratio, the per-bin phase is
  re-accumulated, and an inverse FFT + overlap-add resynthesise the signal (bins past Nyquist are
  dropped, so aliasing is handled in the frequency domain). Parameters `semitones` (±12 = ±1
  octave) and `mix`; the dry path is delayed by the vocoder latency so a dry/wet blend stays
  time-aligned. `effect` category; reports `latency_frames`; `set_params` updates live. Because the
  project keeps no FFT dependency, this ships a self-contained in-place radix-2 FFT
  (`vb_nodes::fft`) verified against a naïve DFT. Primary source: Bernsee, "Pitch Shifting Using
  the Fourier Transform" + `smbPitchShift.cpp` (Wide Open License). Unit-tested for unit-gain
  reconstruction at ratio 1, octave-up/down transposition (energy at 2×/½× the input), a
  pitch-preserving dry mix, silence, and bounded output; `docs/nodes.md` regenerated 🧪
- **Showcase patches for the new effect nodes — Flanged Sweep & Metallic Shift (000214).** The two
  most audibly-demonstrable nodes added this cycle now ship as note-gated factory instruments, so
  a node nobody plays is no longer half-done: **Flanged Sweep** (`examples/flanged_sweep.ron`) — a
  110 Hz saw through the self-sweeping `flanger` (slow deep resonant sweep) into a gated VCA; and
  **Metallic Shift** (`examples/metallic_shift.ron`) — a 220 Hz saw through the `freq-shifter`
  (+250 Hz, full wet) for an inharmonic, bell-like voice. Both registered in the example manifest +
  factory palette with silent-at-rest / audible-on-note behavioural tests. (The EQ/dynamics/allpass
  nodes stay processing utilities — a note-gated "instrument" patch for them would be contrived.)
- **Frequency shifter node (`freq-shifter`, 000213).** A single-sideband (Bode) shifter that
  adds a constant `shift_hz` to every partial — distinct from a pitch shifter (which multiplies by
  a ratio, keeping harmonics integer) and a ring modulator (which keeps both sum and difference
  sidebands). Harmonics become inharmonic, so the sound turns metallic and feedback patches drift
  endlessly. Implemented by forming the analytic signal with an FIR Hilbert transformer (ideal
  `h[n] = 2/(πn)` for odd `n`, Blackman-windowed, 65 taps, group delay 32), rotating it by a
  complex exponential and taking the real part (`y = x·cos − H{x}·sin`); the dry path is delayed
  to match the Hilbert latency so `mix` and `shift_hz = 0` stay phase-aligned. Positive shifts up,
  negative down. Primary sources: Scilab `hilb`, Reilly/Frazer/Boashash 1994. Parameters
  `shift_hz` (−2000..2000) / `mix`, `effect` category. Unit tests verify the zero-shift latency
  passthrough, up/down shift (spectral energy moves to `f ± shift`), **single-sideband mirror
  suppression** (the opposite sideband is ≥20 dB down — proving it is not a ring modulator), the
  dry `mix = 0` path and bounded output; `docs/nodes.md` regenerated.
- **Allpass filter node (`allpass`, 000212).** Unity magnitude at every frequency, only the phase
  is shaped — the building block of phasers (a swept chain of allpass stages), Schroeder reverbs
  and fractional-delay/dispersion effects, now a first-class node (the `phaser` already used
  allpass sections internally). RBJ Audio EQ Cookbook APF biquad (primary source: the W3C/WebAudio
  copy), Direct Form I; because the numerator mirrors the denominator (`b = {a2, a1, a0}`) the
  magnitude is exactly unity at all frequencies, and at the centre frequency the phase is −180° so
  a settled tone there comes out inverted. Parameters `freq_hz` (where the phase swings through
  −180°) and `q` (sharpness), `filter` category. Unit tests verify flat magnitude across the
  spectrum, the centre-frequency inversion, non-passthrough off-centre with preserved level, and
  bounded output; `docs/nodes.md` regenerated.
- **Peaking (bell) EQ node (`peak-eq`, 000211).** A resonant boost or cut of one bell-shaped band
  around a centre frequency, the parametric mid that completes a basic EQ alongside the low/high
  `shelf-eq`. Distinct from `biquad`'s notch (which removes a band outright) — this shapes it by a
  set `gain_db`, with `q` setting the bandwidth. RBJ Audio EQ Cookbook peakingEQ coefficients
  (primary source: the W3C/WebAudio copy), Direct Form I. At `gain_db = 0` it is an exact
  passthrough (`H(z) = 1`); DC and Nyquist always pass at unity (only the band is shaped) and the
  magnitude at the centre is exactly `A² = 10^(gain_db/20)`. Parameters `freq_hz` / `q` /
  `gain_db`, `filter` category. Unit tests verify the 0 dB passthrough, the A² centre boost and
  cut, far-from-centre unity, the `q`-narrower-band behaviour and bounded output; `docs/nodes.md`
  regenerated.
- **Downward expander node (`expander`, 000210).** The mirror of `compressor`: a feed-forward
  dynamics processor that pushes the quiet parts quieter, widening dynamic range (hiss, bleed and
  note tails fall away). Unlike `gate`'s hard open/shut, the attenuation grows smoothly with
  `ratio` — each dB the input sits below `threshold_db` costs `ratio` dB of output, so `ratio = 1`
  is transparent — and `range_db` floors how far the gain can drop so it never fully mutes. Static
  curve (dB) `output = T + R·(x − T)` below threshold, applied gain `(R − 1)·(x − T)`; verified
  against primary sources (MathWorks `expander`, DSP Concepts `DownwardExpanderCore`). Parameters
  `threshold_db` / `ratio` / `attack_ms` / `release_ms` / `range_db`, shared peak-detector idiom
  with `compressor`. Registered in the `dynamics` category (next to `gate`/`limiter`); unit tests
  cover above-threshold passthrough, the ratio-correct attenuation below threshold, deeper-below →
  more expansion, the `range_db` floor, `ratio = 1` transparency and bounded output.
  `docs/nodes.md` regenerated.
- **Shelving EQ node (`shelf-eq`, 000209).** A low- or high-shelf biquad that boosts or cuts one
  end of the spectrum by a fixed `gain_db`, leaving the other end at unity — the bass/treble
  controls of a mixing desk, distinct from `biquad`'s band-removing low-/high-pass and `tone`'s
  single tilt. Parameters `mode` (0 = low shelf, 1 = high shelf), `freq_hz` (corner), `gain_db`
  (−24..24). Coefficients are the RBJ Audio EQ Cookbook shelving biquads (primary source: the
  W3C/WebAudio copy) at a fixed maximally-flat slope `S = 1`, run in Direct Form I; at `gain_db = 0`
  the filter is an exact passthrough (`H(z) = 1`). Unit tests verify the 0 dB passthrough, the
  A² = 10^(dB/20) DC gain of a low-shelf boost/cut, that a high shelf leaves DC at unity, the
  treble lift of a high shelf, and bounded output. Registered in the `filter` category;
  `docs/nodes.md` regenerated from the registry.
- **Flanger effect node (`flanger`, 000207).** A short LFO-modulated delay (~1 ms base, swept a
  few ms) summed with the dry signal to form a sweeping comb filter, plus `feedback`
  (regeneration, `-0.95..=0.95`) that sharpens the notches into the classic resonant, metallic
  flange — negative feedback inverts the reflection for a hollower character. Distinct from the
  existing `chorus`, whose longer (~12 ms) delay and absent feedback give a gentle detune rather
  than a resonant sweep. Parameters `rate_hz` / `depth_ms` / `feedback` / `mix`; registered in the
  `effect` category with unit tests covering dry passthrough, the delayed-tap onset, feedback
  lengthening the decay tail, and bounded output under clamped high feedback. Ranges follow common
  flanger implementations (primary sources: Ableton `abl.dsp.flanger~`, LADSPA dj-flanger,
  Microsoft DirectX flanger). `docs/nodes.md` regenerated from the registry.
- **Three more showcase patches — resonator, bitcrusher, sample-hold (000206).** Completing the
  undemoed-node audit, note-gated and registered in the example manifest + factory palette with
  behavioural tests: **Struck Resonator** (`examples/resonator_pluck.ron`) — a short noise burst
  (gated by a fast `ad` envelope) excites a pitched `resonator` that rings; **Lo-Fi Crush**
  (`examples/lofi_crush.ron`) — a saw through the `bitcrusher` (4-bit, ÷8) and a tame low-pass;
  **Random Steps** (`examples/random_steps.ron`) — a `clock` samples a fast free-running LFO
  through `sample-hold` into a stepped, pseudo-random `ladder` cutoff (the classic sci-fi burble).
- **Three more showcase patches for undemoed nodes (000205).** Continuing the 000204 audit, one
  patch each for three expressive nodes no patch played, note-gated and registered in the example
  manifest + factory palette with behavioural tests: **Granular Cloud** (`examples/granular_cloud.ron`)
  — a saw granulated by `granular`, a slow LFO scanning the grain position (tuned to a short
  read-back delay so the cloud blooms within a few hundred ms rather than seconds); **West-Coast
  Fold** (`examples/west_coast_fold.ron`) — a triangle through the `wavefolder` into a gentle
  `ladder`; **Ring Bell** (`examples/ring_bell.ron`) — a sine `ring-mod`-ulated by a fixed 200 Hz
  carrier for inharmonic bell partials, with a plucky decay envelope.
- **Three showcase patches for previously-undemoed nodes (000204).** An audit found several
  expressive DSP nodes that no example or factory patch actually played — invisible in the
  palette despite being implemented and unit-tested. Added one patch each, note-gated and
  silent-until-note, registered in both the plugin example manifest and the editor factory
  palette, each with a behavioural test (contains the target node, silent at rest, sounds on a
  note): **Plucked String** (`examples/pluck_string.ron`) — the `karplus` Karplus–Strong physical
  model gated per note into a reverb; **Sync Lead** (`examples/sync_lead.ron`) — a `hard-sync`
  oscillator through an envelope-swept `ladder`; **Vowel Pad** (`examples/vowel_pad.ron`) — a saw
  through the `formant` vowel filter whose vowel a slow LFO morphs a→e→i→o→u.
- **Factory patch — Wavetable Pad (`examples/wavetable_pad.ron`, 000203).** The first instrument
  that actually plays the `wavetable` node (added in 000171 with only unit tests — no patch used
  it). Two wavetable oscillators (one up-detuned ~10 cents through a control-rate pitch scale)
  → gentle ladder low-pass → note-gated VCA with a slow pad envelope, a slow LFO sweeping the
  `position` morph (the wavetable's signature timbral movement), then chorus + reverb. A digital
  timbre among the analog clones; silent-until-note, loadable from the editor's factory palette
  as "Wavetable Pad". Added to the plugin example manifest and the factory palette with a
  behavioural test (two wavetable nodes, silent at rest, blooms once the slow envelope opens).

- **Factory patch (`vb-engine`) / Minitaur — LFO → pitch destination (000202).** The real Moog
  Minitaur LFO has **two** independent destinations, each with its own amount knob — **VCO LFO
  AMOUNT** (pitch, for vibrato) and **VCF LFO AMOUNT** (cutoff) — per Moog's Minitaur manual.
  `minitaur_patch()` previously wired only the cutoff leg (000167). It now fans the one LFO out
  to a second control-rate depth+mixer (`cv-attenuvert → cv-add`) summed into the glided note
  frequency feeding both VCOs. The **pitch depth ships at 0** (no vibrato), so the factory bass
  is byte-for-byte unchanged in sound; the routing simply gives the panel a working VCO LFO
  AMOUNT knob, matching the hardware. The Minitaur voice grows from 14 to 16 nodes (a second
  `cv-attenuvert` + `cv-add`); `examples/minitaur.ron` regenerated from the builder. Brings the
  code up to its own spec in [docs/synths/minitaur.md](docs/synths/minitaur.md).

### Changed

- **Library (`vb-library`)** — **track drevo's KV-handle removal (epic #444).** drevo deleted
  its legacy in-memory KV `Drevo` engine and the whole `drevo::db` module (drevo #517), on top
  of the earlier redb removal. Its WAL-native **`drevo::native_service::NativeService`** is now
  the store of record, exposing the same graph CRUD as explicit "parity with `Drevo`" methods.
  This turned the (non-required) `library / drevo` CI job red — `vb-library` still referenced
  `drevo::db::Drevo::open_in_memory()`. Re-ported the whole storage seam onto `NativeService`:
  `Library` now holds a `NativeService` opened with `NativeService::in_memory()`; the calls
  whose drevo return types changed were adapted — `neighbors` / `search_fts` /
  `get_node_by_title` now return plain values (not `Result`), and `get_node` now returns
  `Result<Node>` (a `NodeNotFound` error, not `Ok(None)`); the file backup helpers wrap
  `export_json` / `import_json` with `std::fs` (I/O → `DrevoError::Io`). No public `vb-library`
  API changed; the `library` feature stays off by default, so the required workspace gates are
  untouched. Honors the standing rule to adapt VST_bomb to the current drevo, never to pin an
  old drevo branch.
- **Factory patch (`vb-engine`) / Minitaur** — the two VCOs now **hide `base_hz` from the
  panel**. `base_hz` is a `pitch-osc` resting-only fallback (the frequency held when no note
  plays); a note-gated voice always tracks the played pitch, so the knob did nothing on the
  panel — and the real Minitaur has no per-oscillator base-frequency control (pitch = note +
  VCO2 FREQ + fine tune). `minitaur_patch()` marks param index 1 hidden on both oscillators
  (`NodeEntry::panel_hidden`), so a fresh Minitaur panel shows only real controls (waveform,
  detune); `base_hz` stays editable in the structure inspector. Existing saved patches keep
  their layout — reload the factory patch (or uncheck "on panel" in the inspector) to pick it
  up.
- **Library (`vb-library`)** — **track drevo's redb-backend removal.** drevo retired its redb
  on-disk backend (drevo epic #444), so `drevo::db::Drevo::open`, `Drevo::migrate` and
  `MigrationDirection` no longer exist and the embedded `Drevo` handle is **in-memory only**.
  This left the whole VST_bomb workspace unable to resolve (Cargo resolves an optional
  path-dependency's manifest even when its feature is off), which turned **every** required CI
  check red — `vb-library`'s `drevo` dependency requested the now-missing `redb-backend`
  feature. Adapted `vb-library` to the current drevo: dropped the `redb-backend` feature
  request, removed the unused on-disk `Library::open(path)` and `Library::migrate(...)` (both
  backed by the removed redb API; no caller in the tree used them — every construction is
  `open_in_memory`), and kept the in-memory store plus JSON export/import as the portable
  backup path. Durable persistence would now go through drevo's separate WAL-backed
  `native_service::NativeService` — future work. `cargo test -p vb-library --features library`
  is green.
- **Factory patch / default synth (`vb-engine`)** — the **Moog Minitaur** clone is now a
  **faithful two-VCO voice**. It previously carried **three** oscillators ("two saws plus a
  square sub"), but the real Minitaur has **exactly two VCOs** (VCO1 + VCO2, each saw or
  square; the mixer is a VCO1/VCO2 balance) — there is **no separate sub oscillator**
  (confirmed against Moog's Minitaur / Minitaur Editor manuals). `minitaur_patch()` — which is
  also the plugin's **default patch** and the generator for `examples/minitaur.ron` — now
  builds **VCO1 (saw, unison) + VCO2 (square, one octave down)** through a two-channel
  attenuvert mixer; the octave-down VCO2 reproduces the same fat low end honestly. The
  ladder / two-envelope / LFO / glide topology and the silent-until-a-note behaviour are
  unchanged. See `docs/synths/minitaur.md`.

### Added

- **Editor (`vb-editor`)** — **choose what the panel exposes, from the structure** (Reaktor-style).
  The inspector now lists, per parameter, a **“on panel”** checkbox next to the control-type picker,
  so you pick in the structure which controls the panel shows and which it hides — the inspector
  always lists every parameter, the panel shows only the visible ones (hiding a module's last visible
  control removes the module from the panel). Visibility is presentational (no audio revision) and is
  stored sparsely in the project (`NodeEntry::panel_hidden` = the hidden indices; all-visible stores
  nothing, and older documents load with every control shown). New `PatchView::set_panel_visible` and
  `ParamSlot::panel_visible`; verified in the browser/WASM build.
- **Editor (`vb-editor`)** — a Reaktor-style **Edit / Use split** on the Panel surface. The panel
  used to conflate building and playing — you could drag a module and turn a knob in the same view.
  Now an **“edit UI”** toggle in the toolbar (shown on the Panel surface) switches between two modes:
  in **Use** (the default, so a plugin loads ready to play) the controls are live and the layout is
  locked so a stray drag can't rearrange it; in **Edit** the controls are drawn inert (disabled) so
  laying out the panel can't change the sound and module titles drag to arrange. The panel is
  **layout-only**: a control's type, its “on panel” visibility, and the module's accent colour are
  all set in the structure inspector (Reaktor-style), not on the panel. New `EditorApp::panel_mode` /
  `set_panel_mode` / `panel_layout_editable` and the `PanelMode` enum; verified in the browser/WASM
  build (mode toggle, inert-vs-live controls, locked layout).

### Fixed

- **Editor (`vb-editor`)** — **dragging a module to arrange it in Panel edit mode now works.** Edit
  mode looked dead: grabbing a module's title barely nudged it and then it froze. An un-placed module
  renders in the wrapping flow (egui's Background layer), but the first drag frame *places* it, and it
  then re-renders inside a floating `Area` (a Middle layer). egui forbids a widget id from crossing
  layers, so a drag tracked through the title's own response was dropped the instant the module
  migrated. The drag is now driven centrally from the global pointer (a new `panel_drag` grab state):
  the title only *starts* the gesture, so the move survives the flow→Area handoff. The panel's
  scroll area also no longer drag-scrolls (a background drag could steal a title's grab). Covered by a
  new full egui-driven interaction test (multi-frame pointer drag across the handoff) and verified in
  the browser/WASM build.
- **Editor (`vb-editor`)** — a **right-click on a panel knob no longer moves its value**. The custom
  knob reacted to any drag; with the panel's right-click now reserved for the control-type menu (and
  absent in Use mode), a secondary click on a live knob jumped its value. The knob now turns only on
  a **primary (left) drag** (`dragged_by(PointerButton::Primary)`).

### Changed

- **DSP (`vb-nodes`) / filter (`vcf`)** — the same **exponential (per-octave)** cutoff-modulation
  fix applied to the ladder now covers the multi-mode biquad `vcf`: its control parameter
  `amount_hz` becomes **`amount_oct`** (octaves of swing per unit CV, bi-polar −8…+8, default 3), so
  `cutoff = clamp(base_hz · 2^(amount_oct · cv))`. This makes every filter's cutoff modulation
  consistent (ladder + vcf + vco all speak octaves) and even across the range, and retires the last
  use of `amount_hz`. **Breaking (patch format):** the 4th `vcf` value is now octaves; the two factory
  presets with an explicit `vcf` amount (`auto_wah`, `neutron`) were migrated to preserve their
  envelope-peak cutoff.
- **DSP (`vb-nodes`) / filter (`ladder`)** — the ladder's cutoff modulation is now **exponential
  (per-octave)** instead of linear in Hz. The control-input parameter `amount_hz` (Hz of swing per
  unit CV) is replaced by **`amount_oct`** (octaves of swing per unit CV, bi-polar −8…+8, default 3),
  so `cutoff = clamp(base_hz · 2^(amount_oct · cv))`. This matches how real Moog ladders work — the
  panel control on the **Minitaur** and **Grandmother** is a bi-polar, unit-less **"Envelope Amount"**
  (negative reverses the sweep), and the envelope reaches the cutoff through the filter's exponential
  (~1 V/octave) converter — so a fixed amount moves the cutoff by a fixed number of octaves and sounds
  even across the range instead of collapsing at the low end. It also aligns the ladder with the
  existing `vco` `amount_oct` law. **Breaking (patch format):** the third ladder value is now octaves;
  the built-in default patch and every factory `examples/*.ron` preset were migrated (each amount
  converted to preserve its envelope-peak cutoff, `oct = log2(clamp(base+amount,20,20000)/base)`).
  Hand-saved patches from before will read their old Hz number as octaves for that one knob and clamp
  wide open until re-dialed. Sources: Moog Minitaur & Grandmother manuals (Filter → Envelope Amount).
- **Editor (`vb-editor`)** — control precision. Time (`_s`, `_ms`) and frequency (`_hz`) sliders
  and faders now read on a **logarithmic** scale (`widgets::param_is_log`), so the musically useful
  low end has real travel: on a linear 0–5 s decay fader, 0.03 s and 0.15 s were a couple of pixels
  apart and unreachable; on the log scale a small drag moves cleanly between them (verified in the
  browser/WASM build). Levels (`sustain`, `resonance`), gains (`_db`) and bipolar ranges
  (`detune_semitones`) stay linear. Knobs got a longer full-sweep travel (`KNOB_RANGE_PX` 200 → 320)
  for finer control, and holding **Shift** while turning a knob trims at a quarter speed
  (`KNOB_FINE_FACTOR`). Logarithmic faders/sliders whose range starts at zero (envelope times) now
  set a 1 ms floor (`widgets::LOG_SMALLEST_POSITIVE`): egui's `1e-6` default handed the bottom
  ~45 % of the travel to sub-millisecond times nobody dials and snapped a downward drag to `0.000`
  (the "adsr decay won't leave zero" complaint); with the floor the bottom reaches ~1 ms and small
  decays like 0.01–0.1 s are easy to grab (verified in the browser/WASM build — a drag near the
  bottom lands 0.003 s → 0.010 s cleanly instead of collapsing to zero).
- **Editor (`vb-editor`)** — frequency/time **knobs** now read logarithmically too
  (`widgets::log_normalize` / `log_denormalize`), matching their faders. A 200 Hz ladder cutoff sat
  pinned to the far-left minimum on a linear knob (0.9 % of the sweep) even though the filter was
  wide open; on the log scale it sits ~a third of the way round, where it belongs. Bipolar ranges
  (`amount_oct`, min < 0) stay linear. Verified in the browser/WASM build.
- **Release** — the workspace version now bumps every merge (starting **0.0.2**) so the on-screen
  version moves build to build, instead of staying frozen at `0.0.1` between releases.

### Added

- **Editor (`vb-editor`)** — a **build stamp** next to the version in the toolbar: the short git
  SHA (with a `+` when the working tree is dirty) and the commit date, baked in at compile time by a
  dependency-free `build.rs`. The crate version is frozen at `0.0.1` pre-1.0 and never moves between
  changes, so it could not tell two builds apart — the SHA does, confirming at a glance that a
  freshly-installed bundle is the one actually running. Falls back to `unknown` off a git checkout.
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

- **Library (`vb-editor` / `vb-plugin` / `vb-standalone`)** — the library browser now reads and
  writes saved macro modules in a **stable per-user folder** instead of a relative `modules` path.
  The relative default resolves against the process working directory, which in a plugin is the
  host DAW's — so the browser could never find the user's saved modules and exports landed in an
  unpredictable spot. `module::user_modules_dir()` resolves the platform data folder
  (`~/Library/Application Support/VST_bomb/modules` on macOS, `%APPDATA%\VST_bomb\modules` on
  Windows, `$XDG_DATA_HOME`/`~/.local/share/VST_bomb/modules` elsewhere; relative fallback on WASM),
  and both the plugin and the standalone app point the editor at it via the new
  `EditorApp::set_modules_dir`, so a macro saved in one is browsable in the other.
- **Editor (`vb-editor`)** — the Panel view no longer clips wide modules off the right edge.
  Modules are not uniform width — a four-fader `adsr` is far wider than a one-knob `glide` — so
  laying them out as a fixed number of equal columns overflowed a row of wide modules and cut off
  the last one (the `lfo`'s later controls vanished). The panel now estimates each module's width
  from its control count (`module_est_width`) and greedily packs rows that fit the available width
  (`pack_row_counts`), so a row wraps before it runs off the edge and every control stays visible.
  Verified in the browser (WASM) at 760 px and 1200 px — the `lfo`'s five controls render in full
  at both widths.
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
