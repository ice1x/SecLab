# SecLab node reference

Auto-generated from the registry — do not edit by hand.

## dynamics

### expander

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `threshold_db` — range -80..0, default -40
  - `ratio` — range 1..20, default 2
  - `attack_ms` — range 0.1..100, default 5
  - `release_ms` — range 1..1000, default 50
  - `range_db` — range 0..80, default 40

### gate

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `threshold_db` — range -80..0, default -40
  - `attack_ms` — range 0.1..100, default 1
  - `release_ms` — range 1..1000, default 50

### limiter

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `ceiling_db` — range -24..0, default 0
  - `release_ms` — range 1..1000, default 50

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

### compressor

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `threshold_db` — range -60..0, default -18
  - `ratio` — range 1..20, default 4
  - `attack_ms` — range 0.1..100, default 10
  - `release_ms` — range 1..1000, default 100
  - `makeup_db` — range 0..24, default 0

### fb-delay

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `delay_s` — range 0.001..2, default 0.25
  - `feedback` — range 0..0.95, default 0.4

### flanger

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `rate_hz` — range 0.05..5, default 0.5
  - `depth_ms` — range 0..8, default 3
  - `feedback` — range -0.95..0.95, default 0.5
  - `mix` — range 0..1, default 0.5

### freq-shifter

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `shift_hz` — range -2000..2000, default 0
  - `mix` — range 0..1, default 1

### granular

- **Inputs:** `in` (audio), `position` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `grain_ms` — range 1..1000, default 80
  - `density` — range 0.1..200, default 30
  - `position` — range 0..1, default 0.3
  - `pitch` — range 0.125..8, default 1
  - `spray` — range 0..1, default 0.1

### phaser

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `rate_hz` — range 0.05..10, default 0.5
  - `depth` — range 0..1, default 0.7
  - `feedback` — range 0..0.95, default 0.3
  - `mix` — range 0..1, default 0.5

### ping-pong

- **Inputs:** `in` (audio)
- **Outputs:** `left` (audio), `right` (audio)
- **Parameters:**
  - `delay_s` — range 0.001..2, default 0.25
  - `feedback` — range 0..0.95, default 0.4
  - `mix` — range 0..1, default 0.5

### pitch-shifter

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `semitones` — range -12..12, default 0
  - `mix` — range 0..1, default 1

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

### robotize

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `mix` — range 0..1, default 1

### spectral-blur

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `amount` — range 0..0.999, default 0.9
  - `mix` — range 0..1, default 1

### spectral-freeze

- **Inputs:** `in` (audio), `freeze` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `mix` — range 0..1, default 1

### spectral-gate

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `threshold_db` — range -120..0, default -40
  - `mix` — range 0..1, default 1

### tremolo

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `rate_hz` — range 0.1..20, default 5
  - `depth` — range 0..1, default 0.5

### vocoder

- **Inputs:** `carrier` (audio), `modulator` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `release_ms` — range 1..500, default 15
  - `mix` — range 0..1, default 1

### wavefolder

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `drive` — range 0..10, default 1
  - `symmetry` — range -1..1, default 0
  - `level` — range 0..1, default 1

### waveshaper

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `shape` — range 0..2, default 0
  - `drive` — range 0.1..20, default 2
  - `level` — range 0..1, default 0.8

### whisperize

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `mix` — range 0..1, default 1

### width

- **Inputs:** `left` (audio), `right` (audio)
- **Outputs:** `left` (audio), `right` (audio)
- **Parameters:**
  - `width` — range 0..2, default 1

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

### ahr

- **Inputs:** `gate` (control)
- **Outputs:** `level` (control)
- **Parameters:**
  - `attack_s` — range 0..5, default 0.01
  - `hold_s` — range 0..5, default 0.1
  - `release_s` — range 0..5, default 0.1

### ar

- **Inputs:** `gate` (control)
- **Outputs:** `level` (control)
- **Parameters:**
  - `attack_s` — range 0..5, default 0.01
  - `release_s` — range 0..5, default 0.2

## filter

### allpass

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `freq_hz` — range 20..20000, default 1000
  - `q` — range 0.1..20, default 0.707

### biquad

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `kind` — range 0..3, default 0
  - `cutoff_hz` — range 20..20000, default 1000
  - `q` — range 0.1..10, default 0.707

### dc-blocker

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `r` — range 0.9..0.9999, default 0.995

### formant

- **Inputs:** `in` (audio), `vowel` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `vowel` — range 0..1, default 0
  - `resonance` — range 0.1..4, default 1

### ladder

- **Inputs:** `in` (audio), `cutoff` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 1000
  - `resonance` — range 0..1, default 0.3
  - `amount_oct` — range -8..8, default 3

### peak-eq

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `freq_hz` — range 20..20000, default 1000
  - `q` — range 0.1..20, default 1
  - `gain_db` — range -24..24, default 0

### resonator

- **Inputs:** `in` (audio), `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 220
  - `decay` — range 0..1, default 0.7
  - `structure` — range 0..1, default 0

### shelf-eq

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `mode` — range 0..1, default 0
  - `freq_hz` — range 20..20000, default 1000
  - `gain_db` — range -24..24, default 0

### svf

- **Inputs:** `in` (audio)
- **Outputs:** `lowpass` (audio), `bandpass` (audio), `highpass` (audio)
- **Parameters:**
  - `cutoff_hz` — range 20..20000, default 1000
  - `q` — range 0.1..20, default 0.707

### tone

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `tilt` — range -1..1, default 0
  - `pivot_hz` — range 100..8000, default 700

### vcf

- **Inputs:** `in` (audio), `cutoff` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `kind` — range 0..3, default 0
  - `base_hz` — range 20..20000, default 1000
  - `q` — range 0.1..10, default 0.707
  - `amount_oct` — range -8..8, default 3

## generator

### additive

- **Inputs:** `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 440
  - `partials` — range 1..32, default 8
  - `tilt` — range 0..2, default 1

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

### karplus

- **Inputs:** `gate` (control), `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 220
  - `damping` — range 0..0.9999, default 0.996
  - `tone` — range 0..1, default 0.5

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

### sub-osc

- **Inputs:** `pitch` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 440
  - `octaves` — range 1..2, default 1

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

### wavetable

- **Inputs:** `pitch` (control), `position` (control)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `base_hz` — range 20..20000, default 440
  - `position` — range 0..1, default 0.5

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

### cv-add

- **Inputs:** `a` (control), `b` (control)
- **Outputs:** `out` (control)
- **Parameters:** _(none)_

### cv-attenuvert

- **Inputs:** `in` (control)
- **Outputs:** `out` (control)
- **Parameters:**
  - `scale` — range -2..2, default 1
  - `offset` — range -1..1, default 0

### gain

- **Inputs:** `in` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `gain` — range 0..2, default 1

### max

- **Inputs:** `a` (audio), `b` (audio)
- **Outputs:** `out` (audio)
- **Parameters:** _(none)_

### min

- **Inputs:** `a` (audio), `b` (audio)
- **Outputs:** `out` (audio)
- **Parameters:** _(none)_

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

### xfade

- **Inputs:** `a` (audio), `b` (audio), `mix` (control)
- **Outputs:** `out` (audio)
- **Parameters:** _(none)_

## modulation

### comparator

- **Inputs:** `a` (control), `b` (control)
- **Outputs:** `out` (control)
- **Parameters:**
  - `hysteresis` — range 0..1, default 0

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

### quantizer

- **Inputs:** `in` (control)
- **Outputs:** `out` (control)
- **Parameters:**
  - `ref_hz` — range 20..20000, default 440
  - `divisions` — range 1..48, default 12

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

### pan

- **Inputs:** `in` (audio), `pan` (control)
- **Outputs:** `left` (audio), `right` (audio)
- **Parameters:**
  - `pan` — range -1..1, default 0

### selector

- **Inputs:** `a` (audio), `b` (audio), `c` (audio), `d` (audio)
- **Outputs:** `out` (audio)
- **Parameters:**
  - `input` — range 0..3, default 0

### splitter

- **Inputs:** `in` (audio)
- **Outputs:** `a` (audio), `b` (audio)
- **Parameters:** _(none)_

## sequencing

### clock

- **Inputs:** _(none)_
- **Outputs:** `out` (control)
- **Parameters:**
  - `bpm` — range 20..300, default 120
  - `pulse_width` — range 0.01..0.99, default 0.5

### clock-divider

- **Inputs:** `in` (control)
- **Outputs:** `out` (control)
- **Parameters:**
  - `division` — range 1..16, default 2

### euclid

- **Inputs:** `clock` (control)
- **Outputs:** `gate` (control)
- **Parameters:**
  - `pulses` — range 0..32, default 4
  - `steps` — range 1..32, default 8

### step-sequencer

- **Inputs:** `clock` (control)
- **Outputs:** `cv` (control)
- **Parameters:**
  - `steps` — range 1..8, default 8
  - `step0` — range -1..1, default 0
  - `step1` — range -1..1, default 0
  - `step2` — range -1..1, default 0
  - `step3` — range -1..1, default 0
  - `step4` — range -1..1, default 0
  - `step5` — range -1..1, default 0
  - `step6` — range -1..1, default 0
  - `step7` — range -1..1, default 0
