# Moog Minitaur — clone spec & fidelity audit

The Minitaur is the target for VST_bomb's **default synth** (Epic 14, issue #203) because it
is the simplest of the planned hardware clones: a **monophonic analog bass** with two
oscillators plus a sub, one Moog ladder low-pass, envelopes, an LFO, and glide — no
polyphony, patchbay, spring reverb, or digital engine. This document is the spec the clone is
built against (000162) and the gap list the faithful build (000163) closes.

## Target voice (the real Minitaur)

Signal flow, hardware:

```
VCO1 (saw/square) ┐
VCO2 (saw/square) ├─ mixer (per-osc levels) ─ ladder LP (cutoff, resonance) ─ VCA ─ out
sub  (square −1oct)┘                                    ▲                        ▲
                                                  filter EG × amount        amp EG
LFO ─ (→ cutoff and/or → pitch, with depth)  ── glide on pitch (portamento)
```

Key points that define "sounds like a Minitaur":

- **Two oscillators + sub**, each with its own **level** in the mixer (osc balance is part of
  the sound — e.g. osc1 + sub for a fat low end).
- **Moog ladder** 4-pole low-pass with real **resonance** (up to self-oscillation).
- **Two independent envelopes**: a **filter** contour (with an **amount** knob into cutoff)
  and a separate **amp** envelope. Sharing one envelope is the main way a clone sounds "off".
- **LFO** routable to **cutoff** and **pitch**, each with a **depth**.
- **Glide** (portamento) on the pitch, mono.
- **Silent until a note** (note-gated amp).

## Current clone (`examples/minitaur.ron`) — what it is today

Graph (node id → kind, with stored values):

| id | kind | stored values | role |
| --- | --- | --- | --- |
| 0 | `midi-in` | — | note gate + pitch |
| 1 | `glide` | | portamento on `pitch_hz` |
| 2,3 | `pitch-osc` | detuned | the two main saws |
| 4 | `pitch-osc` | −1 oct | the square sub |
| 5,6 | `add` | — | sum the three oscillators (no levels) |
| 7 | `ladder` | cutoff 200, res 0.7, env_amount 4000 | filter |
| 8 | `adsr` | A 0.003, D 0.12, S 0.5, R 0.18 | envelope |
| 9 | `vca` | bias 0.0, amount 0.34 | note-gated amp |

Wiring: `midi.pitch_hz → glide → each osc.pitch`; `osc2,osc3 → add → add ← osc4`;
`add → ladder.in`; `ladder → vca.in`; `midi.gate → adsr`; **`adsr → ladder.cutoff` and
`adsr → vca.cv`** (one envelope drives both the filter sweep and the amp). Silent at rest
(VCA bias 0), audible on a note — see the `minitaur_is_silent_at_rest_and_sings_on_a_note`
test.

## Gaps to a faithful Minitaur (→ 000163)

1. **One shared envelope.** The filter sweep and the amp use the *same* `adsr`. Split into a
   **filter EG** (own A/D/S/R + amount into `ladder.cutoff`) and an **amp EG** (into
   `vca.cv`). This is the biggest audible difference.
2. **No LFO.** Add an `lfo` routed to `ladder.cutoff` (and optionally osc pitch) with a
   **depth** control.
3. **Bare mixer.** `add` sums the oscillators equally. Give each osc a **level** so the
   osc/sub balance is tunable.

## No new DSP nodes required

Every building block already exists: `pitch-osc`, `glide`, `ladder`, `sub`-style square via a
`pitch-osc`, `adsr`, `vca`, `lfo`, `add`, and crucially **`attenuvert`** (`out = in·scale +
offset`) — the CV utility that provides the **filter-EG amount**, the **LFO depth**, and the
**per-oscillator level** (scale each osc before the sum). So 000163 is assembly + tuning, not
new nodes; 000164 then points `vb_engine::default_patch` (and the editor's starting patch) at
this voice.

## Target graph (for 000163)

```
midi.gate ─┬─ ampEG (adsr) ───────────────── vca.cv
           └─ filtEG (adsr) ─ attenuvert(amount) ─ ladder.cutoff (summed with base)
lfo ─ attenuvert(depth) ─ ladder.cutoff (summed)
midi.pitch ─ glide ─┬─ osc1 ─ attenuvert(lvl) ┐
                    ├─ osc2 ─ attenuvert(lvl) ├─ add ─ ladder.in ─ vca.in ─ out
                    └─ sub  ─ attenuvert(lvl) ┘
```

Pinned values (000134) tune it to a punchy mono bass: low base cutoff with a filter-EG
amount that "pings", short amp attack, moderate sustain, osc1+sub weighted for low end.
