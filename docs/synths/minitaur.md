# Moog Minitaur — clone spec & fidelity audit

The Minitaur is the target for VST_bomb's **default synth** (Epic 14, issue #203) because it
is the simplest of the planned hardware clones: a **monophonic analog bass** with **two
oscillators**, one Moog ladder low-pass, envelopes, an LFO, and glide — no polyphony,
patchbay, spring reverb, or digital engine. This document is the spec the clone is built
against (000162), the gap list the faithful build (000163) closes, and the fidelity
correction (000201) that dropped the phantom third oscillator.

## Target voice (the real Minitaur)

Signal flow, hardware:

```
VCO1 (saw/square) ┐
VCO2 (saw/square) ┴─ mixer (VCO1/VCO2 balance) ─ ladder LP (cutoff, resonance) ─ VCA ─ out
                                                       ▲                        ▲
                                                 filter EG × amount        amp EG
LFO ─ (→ cutoff and/or → pitch, with depth)  ── glide on pitch (portamento)
```

Key points that define "sounds like a Minitaur":

- **Exactly two oscillators** (VCO1 + VCO2), each a **saw or square**, mixed with a
  **VCO1/VCO2 balance** (per-osc levels). There is **no separate sub oscillator** — the "fat
  low end" comes from pulling **VCO2 an octave down** (VCO2 FREQ, ±1 octave / detune ±12 st).
- **Moog ladder** 4-pole low-pass with real **resonance** (up to self-oscillation).
- **Two independent envelopes**: a **filter** contour (with an **amount** knob into cutoff)
  and a separate **amp** envelope. Sharing one envelope is the main way a clone sounds "off".
- **LFO** routable to **cutoff** and **pitch**, each with a **depth**.
- **Glide** (portamento) on the pitch, mono.
- **Silent until a note** (note-gated amp).

Source: Moog *Minitaur* manual and *Minitaur Editor* user's manual (Oscillator Parameters —
VCO1/VCO2 waveform saw/square, VCO2 FREQ ±1 octave, mixer is the two VCOs plus the external
audio input). No dedicated sub-oscillator exists on the instrument.

## Fidelity correction (000201)

The original clone (000162/000163) carried **three** `pitch-osc` nodes — "two saws plus a
square sub" — on the mistaken premise that the Minitaur has a sub oscillator. It does not: the
hardware is a strict **two-VCO** voice. 000201 rebuilds `minitaur_patch()` as two VCOs (VCO1
saw at unison + VCO2 an octave-down square, which reproduces the same fat low end honestly)
through a two-channel mixer. The filter/envelope/LFO/glide topology is unchanged.

## Current clone (`examples/minitaur.ron`) — what it is today

Generated from `vb_engine::minitaur_patch()`. Graph (node id → kind, with stored values):

| id | kind | stored values | role |
| --- | --- | --- | --- |
| 0 | `midi-in` | — | note gate + pitch |
| 1 | `glide` | | portamento on `pitch_hz` |
| 2 | `pitch-osc` | saw, 55 Hz, 0 st | VCO1 |
| 3 | `pitch-osc` | square, 55 Hz, −12 st | VCO2 (octave down) |
| 4,5 | `attenuvert` | 0.9 / 0.8 | VCO1 / VCO2 level (mixer balance) |
| 6 | `add` | — | sum the two VCOs |
| 7 | `ladder` | cutoff 200, res 0.6, amount_oct 4.0 | filter |
| 8 | `adsr` | A .005 D .15 S .3 R .2 | filter EG |
| 9 | `adsr` | A .005 D .10 S .8 R .2 | amp EG |
| 10 | `vca` | bias 0.0, amount 1.0 | note-gated amp |
| 11 | `lfo` | tri, ~5 Hz | modulation source |
| 12 | `cv-attenuvert` | depth 0.0 | VCF LFO AMOUNT → cutoff depth (off) |
| 13 | `cv-add` | — | sum filter EG + LFO into cutoff |
| 14 | `cv-attenuvert` | depth 0.0 | VCO LFO AMOUNT → pitch depth (off) |
| 15 | `cv-add` | — | sum glide + LFO into pitch |

Wiring: `midi.pitch_hz → glide → pitch cv-add ← lfo·pitch-depth`, `pitch cv-add → each
VCO.pitch`; `VCO1 → lvl1`, `VCO2 → lvl2`, `lvl1,lvl2 → add → ladder.in`; `ladder → vca.in`;
`midi.gate → filtEG → cutoff cv-add ← lfo·cutoff-depth`, `cutoff cv-add → ladder.cutoff`;
`midi.gate → ampEG → vca.cv` (two separate envelopes). The one LFO fans out to both depths —
its two destinations (VCO LFO AMOUNT / VCF LFO AMOUNT) as on the hardware. Silent at rest
(VCA bias 0), audible on a note — see the
`minitaur_is_silent_at_rest_and_sings_on_a_note` test.

## Faithful topology (000163 + 000201)

`vb_engine::minitaur_patch()`, from which `examples/minitaur.ron` is generated:

- **Two envelopes:** a filter `adsr` → `ladder.cutoff` (scaled by the ladder's `amount_oct`,
  the bi-polar per-octave "Envelope Amount") and a separate amp `adsr` → `vca.cv`.
- **Two-VCO mix:** each VCO through its own `attenuvert` (VCO1 = 0.9, VCO2 = 0.8) into `add`.
- Glide, silent-until-note (VCA `bias = 0`) preserved.

**LFO → cutoff (000167):** routing the LFO to the cutoff needs to *sum* it with the filter
envelope into the single cutoff CV input, but the audio `add` / `attenuvert` are
**audio-typed** (`SignalType::Audio`, not `compatible_with` `Control`), so they cannot carry a
control signal. 000167 added their control-rate twins — **`cv-add`** and **`cv-attenuvert`**
(same processors, `Control` ports) — and wired
`lfo → cv-attenuvert(depth) → cv-add ← filter-EG → ladder.cutoff`. The LFO **depth is 0 by
default** so the shipped bass is clean, but the routing is there for a filter wobble.

**LFO → pitch (000202):** the Minitaur LFO has a *second* independent destination — **VCO LFO
AMOUNT** — that modulates oscillator pitch for vibrato (Moog Minitaur manual). The clone fans
the same LFO out to a second `cv-attenuvert(pitch-depth) → cv-add ← glide → each VCO.pitch`
(again a control-rate sum, since the glided pitch is a `Control` signal). Like the cutoff
depth, the **pitch depth ships at 0** (no vibrato) — the routing gives the panel a working
VCO LFO AMOUNT knob without changing the shipped bass.

000164 points `vb_engine::default_patch` (and the editor's starting patch) at
`minitaur_patch()`.

## Target graph

```
midi.gate ─┬─ ampEG (adsr) ───────────────────────── vca.cv
           └─ filtEG (adsr) ─ cv-add ─ ladder.cutoff (summed with base)
lfo ─┬─ cv-attenuvert(cutoff-depth) ─ cv-add (summed into cutoff)
     └─ cv-attenuvert(pitch-depth) ─ cv-add (summed into pitch)
midi.pitch ─ glide ─ cv-add ─┬─ VCO1 ─ attenuvert(lvl) ┐
                             └─ VCO2 ─ attenuvert(lvl) ┴─ add ─ ladder.in ─ vca.in ─ out
```

Pinned values (000134) tune it to a punchy mono bass: low base cutoff with a filter-EG amount
that "pings", short amp attack, moderate sustain, VCO2 an octave down for the low end.
