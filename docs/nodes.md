# VST_bomb node reference

Auto-generated from the registry — do not edit by hand.

## dynamics

### vca

- **Inputs:** `in` (audio), `cv` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `bias` — range 0..2, default 1
  - `amount` — range 0..2, default 1

## effect

### fb-delay

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `delay_s` — range 0.001..2, default 0.25
  - `feedback` — range 0..0.95, default 0.4

### reverb

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `room_size` — range 0..1, default 0.5
  - `damping` — range 0..1, default 0.5
  - `mix` — range 0..1, default 0.3

### waveshaper

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `shape` — range 0..2, default 0
  - `drive` — range 0.1..20, default 2
  - `level` — range 0..1, default 0.8

## envelope

### adsr

- **Inputs:** `gate` (control)
- **Outputs:** `level` (control)
- **Parameters:**
  - `attack_s` — range 0..5, default 0.01
  - `decay_s` — range 0..5, default 0.1
  - `sustain` — range 0..1, default 0.7
  - `release_s` — range 0..5, default 0.2

## filter

### biquad

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `kind` — range 0..3, default 0
  - `cutoff_hz` — range 20..20000, default 1000
  - `q` — range 0.1..10, default 0.707

### svf

- **Inputs:** `in` (audio)
- **Outputs:** `lowpass` (audio), `bandpass` (audio), `highpass` (audio)
- **Parameters:**
  - `cutoff_hz` — range 20..20000, default 1000
  - `q` — range 0.1..20, default 0.707

## generator

### noise

- **Inputs:** _(none)_
- **Outputs:** `out` (audio)
- **Parameters:**
  - `kind` — range 0..1, default 0
  - `level` — range 0..1, default 0.5

### oscillator

- **Inputs:** _(none)_
- **Outputs:** `out` (audio)
- **Parameters:**
  - `waveform` — range 0..3, default 0
  - `freq_hz` — range 20..20000, default 440

## input

### audio-in

- **Inputs:** _(none)_
- **Outputs:** `out` (audio)
- **Parameters:**
  - `channel` — range 0..1, default 0

### midi-in

- **Inputs:** _(none)_
- **Outputs:** `gate` (control), `pitch_hz` (control), `velocity` (control)
- **Parameters:** _(none)_

### voice-in

- **Inputs:** _(none)_
- **Outputs:** `gate` (control), `pitch_hz` (control), `velocity` (control)
- **Parameters:**
  - `voice` — range 0..7, default 0

## math

### add

- **Inputs:** `a` (audio), `b` (audio)
- **Outputs:** `out` (audio)
- **Parameters:** _(none)_

### clamp

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `min` — range -1..1, default -1
  - `max` — range -1..1, default 1

### gain

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `gain` — range 0..2, default 1

### mix

- **Inputs:** `a` (audio), `b` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `t` — range 0..1, default 0.5

### mul

- **Inputs:** `a` (audio), `b` (audio)
- **Outputs:** `out` (audio)
- **Parameters:** _(none)_

## modulation

### lfo

- **Inputs:** _(none)_
- **Outputs:** `out` (control)
- **Parameters:**
  - `waveform` — range 0..3, default 0
  - `sync` — range 0..1, default 0
  - `freq_hz` — range 0.01..50, default 2
  - `bpm` — range 20..300, default 120
  - `division` — range 0..4, default 2

## output

### audio-out

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `channel` — range 0..1, default 0

## routing

### merger

- **Inputs:** `a` (audio), `b` (audio), `c` (audio), `d` (audio)
- **Outputs:** `out` (audio)
- **Parameters:** _(none)_

### selector

- **Inputs:** `a` (audio), `b` (audio), `c` (audio), `d` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `input` — range 0..3, default 0

### splitter

- **Inputs:** `in` (audio)
- **Outputs:** `a` (audio), `b` (audio)
- **Parameters:** _(none)_
