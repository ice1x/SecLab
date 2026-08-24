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

### bitcrusher

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `bits` — range 1..16, default 8
  - `downsample` — range 1..64, default 1
  - `mix` — range 0..1, default 1

### chorus

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `rate_hz` — range 0.05..10, default 0.8
  - `depth_ms` — range 0..10, default 4
  - `mix` — range 0..1, default 0.5

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

### ring-mod

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `freq_hz` — range 1..8000, default 200
  - `mix` — range 0..1, default 1

### tremolo

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `rate_hz` — range 0.1..20, default 5
  - `depth` — range 0..1, default 0.5

### waveshaper

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `shape` — range 0..2, default 0
  - `drive` — range 0.1..20, default 2
  - `level` — range 0..1, default 0.8

## envelope

### ad

- **Inputs:** `gate` (control)
- **Outputs:** `level` (control)
- **Parameters:**
  - `attack_s` — range 0..5, default 0.005
  - `decay_s` — range 0..5, default 0.15

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

### ladder

- **Inputs:** `in` (audio), `cutoff` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 1000
  - `resonance` — range 0..1, default 0.3
  - `amount_hz` — range -20000..20000, default 4000

### svf

- **Inputs:** `in` (audio)
- **Outputs:** `lowpass` (audio), `bandpass` (audio), `highpass` (audio)
- **Parameters:**
  - `cutoff_hz` — range 20..20000, default 1000
  - `q` — range 0.1..20, default 0.707

### vcf

- **Inputs:** `in` (audio), `cutoff` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `kind` — range 0..3, default 0
  - `base_hz` — range 20..20000, default 1000
  - `q` — range 0.1..10, default 0.707
  - `amount_hz` — range -20000..20000, default 4000

## generator

### fm-op

- **Inputs:** `pitch` (control), `mod` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 440
  - `ratio` — range 0..16, default 1
  - `index` — range 0..24, default 0

### hard-sync

- **Inputs:** `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 440
  - `sync_ratio` — range 1..8, default 2

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

### pitch-osc

- **Inputs:** `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `waveform` — range 0..3, default 1
  - `base_hz` — range 20..20000, default 440
  - `detune_semitones` — range -24..24, default 0

### pwm

- **Inputs:** `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 440
  - `width` — range 0.01..0.99, default 0.5

### supersaw

- **Inputs:** `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 440
  - `detune_semitones` — range 0..2, default 0.3
  - `voices` — range 1..9, default 7

### vco

- **Inputs:** `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `waveform` — range 0..3, default 0
  - `base_hz` — range 20..20000, default 440
  - `amount_oct` — range -4..4, default 1

## input

### audio-in

- **Inputs:** _(none)_
- **Outputs:** `out` (audio)
- **Parameters:**
  - `channel` — range 0..1, default 0

### key-gate

- **Inputs:** _(none)_
- **Outputs:** `gate` (control)
- **Parameters:**
  - `note` — range 0..127, default 36

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

### attenuvert

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `scale` — range -2..2, default 1
  - `offset` — range -1..1, default 0

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

### rectify

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:** _(none)_

### sub

- **Inputs:** `a` (audio), `b` (audio)
- **Outputs:** `out` (audio)
- **Parameters:** _(none)_

## modulation

### env-follower

- **Inputs:** `in` (audio)
- **Outputs:** `out` (control)
- **Parameters:**
  - `attack_s` — range 0..1, default 0.005
  - `release_s` — range 0..2, default 0.05

### glide

- **Inputs:** `in` (control)
- **Outputs:** `out` (control)
- **Parameters:**
  - `glide_ms` — range 0..5000, default 50

### lfo

- **Inputs:** _(none)_
- **Outputs:** `out` (control)
- **Parameters:**
  - `waveform` — range 0..3, default 0
  - `sync` — range 0..1, default 0
  - `freq_hz` — range 0.01..50, default 2
  - `bpm` — range 20..300, default 120
  - `division` — range 0..4, default 2

### sample-hold

- **Inputs:** `in` (control), `trig` (control)
- **Outputs:** `out` (control)
- **Parameters:** _(none)_

### slew

- **Inputs:** `in` (control)
- **Outputs:** `out` (control)
- **Parameters:**
  - `rise_s` — range 0..5, default 0.05
  - `fall_s` — range 0..5, default 0.05

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
