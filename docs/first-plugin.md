# Build your first plugin

This walkthrough builds a small subtractive synth voice — **oscillator → filter →
envelope → output** — and turns it into a loadable plugin. The finished patch ships as
[`examples/first_synth.ron`](../examples/first_synth.ron), and it is covered by a test
that loads it, validates it, and compiles it into a running engine, so what you build here
is known to work.

## What we're building

```
 oscillator ──▶ filter (biquad) ──▶ VCA ──▶ output (gain)
                                     ▲
 MIDI in ──gate──▶ envelope (ADSR) ──┘ level
```

Signal flows left to right. The **oscillator** makes a raw tone, the **filter** shapes its
timbre, and the **VCA** (a voltage-controlled amplifier) sets its loudness from moment to
moment. The **envelope** drives that loudness: a MIDI note opens its *gate*, the ADSR turns
that into a rising-then-falling *level*, and the VCA multiplies the audio by it — so a note
fades in and out instead of clicking on. The **gain** node is the plugin's output.

> **Pitch note.** In this first patch the oscillator runs at a fixed frequency; the MIDI
> note gates the envelope but does not (yet) set the pitch — an absolute-pitch oscillator
> input is future work. To *modulate* pitch from a control signal today, see the
> [`vco`](adding-a-node.md) node and the `vibrato` example.

## Build it in the editor

Open the editor (`cargo run -p vb-standalone`) and place six nodes on the canvas — the
node palette groups them by category:

| Node | Category | Role |
|------|----------|------|
| `oscillator` | generator | the raw tone |
| `biquad` | filter | shapes the timbre |
| `midi-in` | input | note gate from the host |
| `adsr` | envelope | gate → amplitude contour |
| `vca` | dynamics | applies the envelope to the audio |
| `gain` | math | the output |

Then wire them by dragging from an output port to an input port (SecLab only lets you
connect matching signal types, and refuses wires that would make a cycle):

1. `oscillator.out` → `biquad.in`
2. `biquad.out` → `vca.in`
3. `midi-in.gate` → `adsr.gate`
4. `adsr.level` → `vca.cv`
5. `vca.out` → `gain.in`

That is the whole voice. `gain.out` is the terminal audio node — the editor plays the
patch's terminal node, so you'll hear it as you build.

## Dial it in

Select a node to edit its parameters in the inspector. Good starting points:

- **oscillator** — `waveform` to `saw` (a bright, harmonically rich source the filter can
  work on), `freq_hz` to taste.
- **biquad** — `kind` `lowpass`, `cutoff_hz` somewhere mid-range (e.g. 1–2 kHz) so the
  filter audibly darkens the saw, `q` for a little resonance.
- **adsr** — a short `attack_s`, moderate `decay_s`, `sustain` around 0.7, a `release_s`
  long enough to hear the tail.
- **vca** — set `bias` to 0 and `amount` to 1 so the envelope has *full* control: silent
  with no note, opening only while the envelope is up. (With the default `bias` of 1 the
  voice passes sound even without a note — fine for testing, less expressive.)

Every parameter edit is undoable, and the editor re-auditions the patch as you go.

## Save and export

The editor writes the patch as a `.ron` project file — the same format as the committed
[`examples/first_synth.ron`](../examples/first_synth.ron). To run it as a plugin, point the
build at your patch and compile the CLAP:

```bash
# Build the CLAP (a libvb_plugin cdylib; rename to seclab.clap to load it in a host).
cargo build -p vb-plugin --release

# The plugin loads the patch named by VB_PATCH, falling back to a built-in default.
VB_PATCH=examples/first_synth.ron <your host or clap-validator> …
```

Validate the bundle with `cargo xtask validate` (this is what CI runs). VST3/AU builds go
through clap-wrapper — see the plugin crate docs.

That's the round trip: wire a patch, shape it, and load it in a host. From here, try
swapping the biquad for a [`vcf`](adding-a-node.md) so an LFO can sweep the cutoff, or add
a second oscillator for a fatter, detuned voice (see the `unison` example).

## See also

- [Node reference](nodes.md) — every built-in node, its ports and parameters.
- [Adding your own node](adding-a-node.md) — extend the library with custom DSP.
- [`examples/`](../examples) — small, tested starter patches.
