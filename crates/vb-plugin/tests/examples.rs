//! Starter example projects: each committed patch under `examples/` must load, validate,
//! and build a working engine with an audio sink. These are the "first patch" content a
//! user opens, so a regression that breaks one (a renamed node, a changed port) fails CI.

use vb_nodes::{BuildContext, Registry};
use vb_plugin::engine_builder::engine_from_doc;

/// (name, committed RON) for every shipped example.
const EXAMPLES: &[(&str, &str)] = &[
    ("synth", include_str!("../../../examples/synth.ron")),
    ("delay", include_str!("../../../examples/delay.ron")),
    ("unison", include_str!("../../../examples/unison.ron")),
    ("tremolo", include_str!("../../../examples/tremolo.ron")),
    (
        "filter_sweep",
        include_str!("../../../examples/filter_sweep.ron"),
    ),
    ("vibrato", include_str!("../../../examples/vibrato.ron")),
    (
        "first_synth",
        include_str!("../../../examples/first_synth.ron"),
    ),
    (
        "ladder_bass",
        include_str!("../../../examples/ladder_bass.ron"),
    ),
    ("mono_lead", include_str!("../../../examples/mono_lead.ron")),
    ("minimoog", include_str!("../../../examples/minimoog.ron")),
    ("minitaur", include_str!("../../../examples/minitaur.ron")),
    ("neutron", include_str!("../../../examples/neutron.ron")),
    ("edge", include_str!("../../../examples/edge.ron")),
    ("tb303", include_str!("../../../examples/tb303.ron")),
    ("sh101", include_str!("../../../examples/sh101.ron")),
    ("juno60", include_str!("../../../examples/juno60.ron")),
    (
        "grandmother",
        include_str!("../../../examples/grandmother.ron"),
    ),
    ("maul", include_str!("../../../examples/maul.ron")),
    ("minilogue", include_str!("../../../examples/minilogue.ron")),
    ("drumkit", include_str!("../../../examples/drumkit.ron")),
    (
        "supersaw_lead",
        include_str!("../../../examples/supersaw_lead.ron"),
    ),
    ("fm_ep", include_str!("../../../examples/fm_ep.ron")),
    ("auto_wah", include_str!("../../../examples/auto_wah.ron")),
    (
        "super_bass",
        include_str!("../../../examples/super_bass.ron"),
    ),
    (
        "organ_phaser",
        include_str!("../../../examples/organ_phaser.ron"),
    ),
    (
        "stair_glide",
        include_str!("../../../examples/stair_glide.ron"),
    ),
    (
        "clock_poly",
        include_str!("../../../examples/clock_poly.ron"),
    ),
    (
        "euclid_pulse",
        include_str!("../../../examples/euclid_pulse.ron"),
    ),
    ("step_lead", include_str!("../../../examples/step_lead.ron")),
    ("auto_pan", include_str!("../../../examples/auto_pan.ron")),
    (
        "stereo_lead",
        include_str!("../../../examples/stereo_lead.ron"),
    ),
    ("pan_perc", include_str!("../../../examples/pan_perc.ron")),
    (
        "wavetable_pad",
        include_str!("../../../examples/wavetable_pad.ron"),
    ),
    (
        "pluck_string",
        include_str!("../../../examples/pluck_string.ron"),
    ),
    ("sync_lead", include_str!("../../../examples/sync_lead.ron")),
    ("vowel_pad", include_str!("../../../examples/vowel_pad.ron")),
    (
        "granular_cloud",
        include_str!("../../../examples/granular_cloud.ron"),
    ),
    (
        "west_coast_fold",
        include_str!("../../../examples/west_coast_fold.ron"),
    ),
    ("ring_bell", include_str!("../../../examples/ring_bell.ron")),
    (
        "resonator_pluck",
        include_str!("../../../examples/resonator_pluck.ron"),
    ),
    (
        "lofi_crush",
        include_str!("../../../examples/lofi_crush.ron"),
    ),
    (
        "random_steps",
        include_str!("../../../examples/random_steps.ron"),
    ),
    (
        "flanged_sweep",
        include_str!("../../../examples/flanged_sweep.ron"),
    ),
    (
        "metallic_shift",
        include_str!("../../../examples/metallic_shift.ron"),
    ),
    (
        "octave_stack",
        include_str!("../../../examples/octave_stack.ron"),
    ),
];

#[test]
fn every_example_loads_validates_and_builds_an_engine() {
    let registry = Registry::with_builtins();
    let ctx = BuildContext::default();

    for (name, ron) in EXAMPLES {
        // Loads and validates (open_ron runs the structural validator).
        let doc = vb_project::open_ron(ron)
            .unwrap_or_else(|e| panic!("example '{name}' failed to load/validate: {e}"));
        assert!(doc.is_valid(), "example '{name}' is not valid");

        // Builds a real engine at a typical block size, with an audio sink to play.
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256)
            .unwrap_or_else(|e| panic!("example '{name}' failed to build an engine: {e:?}"));
        assert!(sink.is_some(), "example '{name}' has no audio sink");

        // And processes a block without panicking or producing non-finite samples.
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let out = engine
            .node_output(sink.unwrap())
            .and_then(|b| b.channel(0))
            .expect("sink output");
        assert!(
            out.iter().all(|s| s.is_finite()),
            "example '{name}' produced non-finite audio"
        );
    }
}

#[test]
fn ladder_bass_is_silent_at_rest_and_sings_on_a_note() {
    // The ladder-bass factory patch: an envelope-swept resonant ladder gated by a note.
    // Its stored VCA bias = 0 keeps it silent until a note opens the envelope.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/ladder_bass.ron");
    let doc = vb_project::open_ron(ron).expect("ladder_bass loads");

    let peak = |ctx: &BuildContext| {
        let (mut engine, sink) = engine_from_doc(&doc, &registry, ctx, 256).expect("builds");
        let sink = sink.expect("has a sink");
        // A few blocks so the filter/amp envelope has time to open.
        let mut p = 0.0_f32;
        for _ in 0..8 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 256,
            });
            let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
            p = p.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
        p
    };

    let at_rest = peak(&BuildContext::default());
    assert!(
        at_rest < 1e-6,
        "ladder_bass must be silent at rest: {at_rest}"
    );

    let ctx = BuildContext::default();
    ctx.notes.note_on(45, 100); // ~110 Hz
    let on_note = peak(&ctx);
    assert!(
        on_note > 0.01,
        "ladder_bass must sound on a note: {on_note}"
    );
}

#[test]
fn minitaur_is_silent_at_rest_and_sings_on_a_note() {
    // The Minitaur factory patch — the voice slated to become the default synth (Epic 14).
    // Baseline behaviour the faithful rebuild (000163) must preserve: a note-gated mono bass
    // that is silent until played (stored VCA bias = 0) and audible once a note opens the
    // envelope. See docs/synths/minitaur.md.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/minitaur.ron");
    let doc = vb_project::open_ron(ron).expect("minitaur loads");

    let peak = |ctx: &BuildContext| {
        let (mut engine, sink) = engine_from_doc(&doc, &registry, ctx, 256).expect("builds");
        let sink = sink.expect("has a sink");
        let mut p = 0.0_f32;
        for _ in 0..8 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 256,
            });
            let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
            p = p.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
        p
    };

    let at_rest = peak(&BuildContext::default());
    assert!(at_rest < 1e-6, "minitaur must be silent at rest: {at_rest}");

    let ctx = BuildContext::default();
    ctx.notes.note_on(40, 100); // ~82 Hz — a low bass note
    let on_note = peak(&ctx);
    assert!(on_note > 0.01, "minitaur must sound on a note: {on_note}");
}

#[test]
fn edge_is_a_percussive_hit_that_decays() {
    // The Behringer Edge clone (analog percussion): tone + noise → resonant ladder → VCA gated
    // by a one-shot AD envelope. A note fires a short HIT that decays back to near-silence even
    // while held (unlike the sustained Minitaur) — that is what makes it percussion.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/edge.ron");
    let doc = vb_project::open_ron(ron).expect("edge loads");

    // Render `blocks` blocks after firing a note, returning (early peak, late peak).
    let ctx = BuildContext::default();
    ctx.notes.note_on(48, 120);
    let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256).expect("builds");
    let sink = sink.expect("has a sink");
    let block_peak = |engine: &mut vb_runtime::RtEngine| {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        engine
            .node_output(sink)
            .and_then(|b| b.channel(0))
            .unwrap()
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()))
    };

    // The attack (~2 ms) opens within the first blocks: the hit is loud.
    let mut early = 0.0_f32;
    for _ in 0..3 {
        early = early.max(block_peak(&mut engine));
    }
    assert!(early > 0.05, "the hit must be audible: {early}");

    // After the ~120 ms decay (~22 blocks) it has fallen back toward silence — sample the
    // peak of the final block.
    let mut late = 0.0_f32;
    for _ in 0..40 {
        late = block_peak(&mut engine);
    }
    assert!(
        late < early * 0.2,
        "the hit must decay (percussion): early {early}, late {late}"
    );
}

#[test]
fn tb303_is_silent_at_rest_squelches_on_a_note_and_decays() {
    // The acid-bass factory patch: a single saw through a high-resonance ladder, gated by a
    // note; silent at rest (VCA bias 0), a resonant "squelch" on a note, decaying to near
    // silence as the amp AD envelope runs out (a plucky 303 note, not a sustained tone).
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/tb303.ron");
    let doc = vb_project::open_ron(ron).expect("tb303 loads");

    let ctx = BuildContext::default();
    let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256).expect("builds");
    let sink = sink.expect("has a sink");
    let mut block_peak = || {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        engine
            .node_output(sink)
            .and_then(|b| b.channel(0))
            .unwrap()
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()))
    };

    // Silent at rest.
    let mut rest = 0.0_f32;
    for _ in 0..4 {
        rest = rest.max(block_peak());
    }
    assert!(rest < 1e-6, "tb303 must be silent at rest: {rest}");

    // A note squelches: audible in the first blocks after the gate.
    ctx.notes.note_on(45, 120);
    let mut early = 0.0_f32;
    for _ in 0..8 {
        early = early.max(block_peak());
    }
    assert!(early > 0.01, "tb303 must sound on a note: {early}");

    // The amp AD (~0.4 s) runs out even while the note is held → the tail decays away.
    let mut tail = 0.0_f32;
    for _ in 0..160 {
        tail = block_peak();
    }
    assert!(
        tail < early * 0.25,
        "the 303 note must decay (plucky), not sustain: early {early}, tail {tail}"
    );
}

#[test]
fn sh101_is_silent_at_rest_and_sustains_a_note() {
    // The SH-101 factory patch: saw + pulse + square sub + a hint of noise → resonant ladder,
    // one ADSR fanned to filter and amp. Silent at rest (VCA bias 0); a held note is audible
    // and SUSTAINS (the ADSR has a sustain level, unlike the plucky 303/Edge).
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/sh101.ron");
    let doc = vb_project::open_ron(ron).expect("sh101 loads");

    let ctx = BuildContext::default();
    let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256).expect("builds");
    let sink = sink.expect("has a sink");
    let mut block_peak = || {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        engine
            .node_output(sink)
            .and_then(|b| b.channel(0))
            .unwrap()
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()))
    };

    let mut rest = 0.0_f32;
    for _ in 0..4 {
        rest = rest.max(block_peak());
    }
    assert!(rest < 1e-6, "sh101 must be silent at rest: {rest}");

    ctx.notes.note_on(45, 110);
    // Let the envelope reach sustain, then confirm the note is still ringing.
    let mut held = 0.0_f32;
    for _ in 0..40 {
        held = block_peak();
    }
    assert!(
        held > 0.01,
        "a held sh101 note must sustain audibly: {held}"
    );
}

#[test]
fn juno60_is_silent_at_rest_and_polyphonic_notes_sound_through_the_chorus() {
    // The Juno-60 factory patch: a 4-voice polyphonic pad (each voice a saw+pulse+sub DCO →
    // resonant ladder → note-gated vca), summed into the signature chorus. Silent at rest;
    // POLY notes (via the voice allocator's PolyNoteSource, not the mono midi-in) open the
    // voices and are audible through the chorus.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/juno60.ron");
    let doc = vb_project::open_ron(ron).expect("juno60 loads");

    let ctx = BuildContext::default();
    let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256).expect("builds");
    let sink = sink.expect("has a sink");
    let mut block_peak = || {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        engine
            .node_output(sink)
            .and_then(|b| b.channel(0))
            .unwrap()
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()))
    };

    let mut rest = 0.0_f32;
    for _ in 0..4 {
        rest = rest.max(block_peak());
    }
    assert!(rest < 1e-6, "juno60 must be silent at rest: {rest}");

    // Play a three-note chord into the polyphonic source.
    ctx.poly.note_on(48, 100);
    ctx.poly.note_on(52, 100);
    ctx.poly.note_on(55, 100);
    let mut chord = 0.0_f32;
    for _ in 0..40 {
        chord = chord.max(block_peak());
    }
    assert!(chord > 0.01, "a held chord must sound: {chord}");
}

#[test]
fn mono_lead_tracks_the_played_note_pitch() {
    // The mono-lead factory patch is playable: its pitch-tracking oscillator follows
    // midi-in.pitch_hz, so different notes come out at different pitches. Assert the
    // whole engine — midi → pitch-osc → ladder → vca — produces a higher output
    // frequency for a higher note.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/mono_lead.ron");
    let doc = vb_project::open_ron(ron).expect("mono_lead loads");

    // Zero crossings over a rendered tail — proportional to the output frequency.
    let crossings_for = |note: u8| {
        let ctx = BuildContext::default();
        ctx.notes.note_on(note, 110);
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 512).expect("builds");
        let sink = sink.expect("has a sink");
        // Let the envelope open, then count crossings over a settled block.
        for _ in 0..6 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 512,
            });
        }
        let out = engine
            .node_output(sink)
            .and_then(|b| b.channel(0))
            .unwrap()
            .to_vec();
        out.windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count()
    };

    let low = crossings_for(45); // ~110 Hz
    let high = crossings_for(69); // 440 Hz, two octaves up
    assert!(
        high > low * 2,
        "a higher note must sound higher: {low} vs {high} crossings"
    );
}

#[test]
fn minimoog_is_a_playable_three_oscillator_voice() {
    // The Minimoog Model D factory patch: three detuned pitch-tracking oscillators mixed
    // into an envelope-swept ladder and a note-gated VCA. Silent at rest, plays on a note,
    // and tracks the played pitch — a real playable clone.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/minimoog.ron");
    let doc = vb_project::open_ron(ron).expect("minimoog loads");
    // Three oscillators is the essence of the Model D voice.
    assert_eq!(
        doc.nodes
            .iter()
            .filter(|e| e.node.kind == "pitch-osc")
            .count(),
        3,
        "Model D has three oscillators"
    );

    let render = |note: Option<u8>| {
        let ctx = BuildContext::default();
        if let Some(n) = note {
            ctx.notes.note_on(n, 100);
        }
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 512).expect("builds");
        let sink = sink.expect("has a sink");
        let mut peak = 0.0_f32;
        let mut crossings = 0usize;
        for _ in 0..6 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 512,
            });
            let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
            peak = peak.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
            crossings = out
                .windows(2)
                .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
                .count();
        }
        (peak, crossings)
    };

    let (at_rest, _) = render(None);
    assert!(at_rest < 1e-6, "minimoog must be silent at rest: {at_rest}");

    let (low_peak, low_cross) = render(Some(45)); // ~110 Hz
    let (_, high_cross) = render(Some(69)); // 440 Hz
    assert!(low_peak > 0.05, "minimoog must sound on a note: {low_peak}");
    assert!(
        high_cross > low_cross * 2,
        "a higher note must sound higher: {low_cross} vs {high_cross}"
    );
}

#[test]
fn minitaur_is_a_playable_bass_with_glide() {
    // The Minitaur factory patch: two VCOs (VCO1 saw + VCO2 an octave-down square — the real
    // Minitaur has exactly two oscillators, no separate sub) through a glide (portamento) stage
    // into an env-swept ladder and a note-gated VCA. Silent at rest, plays on a note, tracks
    // pitch, and carries a glide node.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/minitaur.ron");
    let doc = vb_project::open_ron(ron).expect("minitaur loads");
    assert_eq!(
        doc.nodes.iter().filter(|e| e.node.kind == "glide").count(),
        1,
        "Minitaur has portamento"
    );
    assert_eq!(
        doc.nodes
            .iter()
            .filter(|e| e.node.kind == "pitch-osc")
            .count(),
        2,
        "two VCOs (no separate sub)"
    );

    let render = |note: Option<u8>| {
        let ctx = BuildContext::default();
        if let Some(n) = note {
            ctx.notes.note_on(n, 100);
        }
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 512).expect("builds");
        let sink = sink.expect("has a sink");
        let (mut peak, mut crossings) = (0.0_f32, 0usize);
        for _ in 0..6 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 512,
            });
            let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
            peak = peak.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
            crossings = out
                .windows(2)
                .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
                .count();
        }
        (peak, crossings)
    };

    let (at_rest, _) = render(None);
    assert!(at_rest < 1e-6, "minitaur must be silent at rest: {at_rest}");
    let (low_peak, low_cross) = render(Some(33)); // low bass note
    let (_, high_cross) = render(Some(57)); // two octaves up
    assert!(low_peak > 0.05, "minitaur must sound on a note: {low_peak}");
    assert!(
        high_cross > low_cross * 2,
        "a higher note must sound higher: {low_cross} vs {high_cross}"
    );
}

#[test]
fn neutron_is_a_playable_voice_with_overdrive_and_delay() {
    // The Behringer Neutron factory patch: two oscillators through a multimode filter,
    // an overdrive (waveshaper) and an in-voice delay, gated by the note envelope.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/neutron.ron");
    let doc = vb_project::open_ron(ron).expect("neutron loads");
    // The overdrive + in-voice delay are what set the Neutron voice apart.
    for kind in ["waveshaper", "fb-delay", "vcf"] {
        assert!(
            doc.nodes.iter().any(|e| e.node.kind == kind),
            "neutron voice has a {kind}"
        );
    }

    let render = |note: Option<u8>| {
        let ctx = BuildContext::default();
        if let Some(n) = note {
            ctx.notes.note_on(n, 100);
        }
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 512).expect("builds");
        let sink = sink.expect("has a sink");
        let mut peak = 0.0_f32;
        for _ in 0..8 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 512,
            });
            let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
            peak = peak.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
            assert!(out.iter().all(|s| s.is_finite()), "finite audio");
        }
        peak
    };

    assert!(render(None) < 1e-6, "neutron must be silent at rest");
    assert!(render(Some(48)) > 0.05, "neutron must sound on a note");
}

#[test]
fn grandmother_is_a_playable_voice_with_reverb() {
    // The Moog Grandmother factory patch: two oscillators through an env-swept ladder and
    // a note-gated VCA into a spring-reverb-style reverb tail.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/grandmother.ron");
    let doc = vb_project::open_ron(ron).expect("grandmother loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "reverb"),
        "grandmother has a reverb voice"
    );

    let peak = |note: Option<u8>| {
        let ctx = BuildContext::default();
        if let Some(n) = note {
            ctx.notes.note_on(n, 100);
        }
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 512).expect("builds");
        let sink = sink.expect("has a sink");
        let mut p = 0.0_f32;
        for _ in 0..8 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 512,
            });
            let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
            assert!(out.iter().all(|s| s.is_finite()), "finite audio");
            p = p.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
        p
    };

    assert!(peak(None) < 1e-6, "grandmother must be silent at rest");
    assert!(peak(Some(52)) > 0.05, "grandmother must sound on a note");
}

#[test]
fn maul_is_a_three_band_distortion_effect() {
    // The fxpansion-Maul-style multiband distortion: a rich source split into three bands
    // (two `svf` crossovers), each driven by its own `waveshaper`, then recombined. An
    // effect demo — it has a built-in source, so it sounds without a note.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/maul.ron");
    let doc = vb_project::open_ron(ron).expect("maul loads");
    assert_eq!(
        doc.nodes
            .iter()
            .filter(|e| e.node.kind == "waveshaper")
            .count(),
        3,
        "three bands, three waveshapers"
    );
    assert_eq!(
        doc.nodes.iter().filter(|e| e.node.kind == "svf").count(),
        2,
        "two crossovers split three bands"
    );

    let (mut engine, sink) =
        engine_from_doc(&doc, &registry, &BuildContext::default(), 512).expect("builds");
    let sink = sink.expect("has a sink");
    let mut peak = 0.0_f32;
    for _ in 0..4 {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 512,
        });
        let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
        assert!(out.iter().all(|s| s.is_finite()), "finite audio");
        peak = peak.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
    }
    assert!(peak > 0.05, "the distortion effect is audible: {peak}");
}

#[test]
fn minilogue_is_polyphonic() {
    // The Korg Minilogue XD factory patch: four independent voice chains, each fed by a
    // `voice-in` node bound to its own slot, summed into a reverb. Polyphony proof: a
    // three-note chord fills three voices and sums to more energy than a single note.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/minilogue.ron");
    let doc = vb_project::open_ron(ron).expect("minilogue loads");
    assert_eq!(
        doc.nodes
            .iter()
            .filter(|e| e.node.kind == "voice-in")
            .count(),
        4,
        "four-voice polyphony"
    );

    let rms = |notes: &[u8]| {
        let ctx = BuildContext::default();
        for &n in notes {
            ctx.poly.note_on(n, 100);
        }
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 512).expect("builds");
        let sink = sink.expect("has a sink");
        // Let the envelopes open, then measure a settled block.
        let mut last = Vec::new();
        for _ in 0..8 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 512,
            });
            last = engine
                .node_output(sink)
                .and_then(|b| b.channel(0))
                .unwrap()
                .to_vec();
            assert!(last.iter().all(|s| s.is_finite()), "finite audio");
        }
        (last.iter().map(|s| s * s).sum::<f32>() / last.len() as f32).sqrt()
    };

    let silent = rms(&[]);
    assert!(silent < 1e-6, "minilogue must be silent at rest: {silent}");

    let one = rms(&[60]);
    let chord = rms(&[60, 64, 67]);
    assert!(one > 0.01, "a single note sounds: {one}");
    assert!(
        chord > one * 1.4,
        "a chord sums several voices, louder than one note: {one} vs {chord}"
    );
}

#[test]
fn drumkit_triggers_each_voice_by_its_note_and_plays_them_together() {
    // The TR-style drum kit: kick (36) / snare (38) / hat (42), each a `key-gate` firing
    // its own synth voice. Because the gates read the polyphonic note source, several
    // drums can sound at once — a real kit, not a mono demo.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/drumkit.ron");
    let doc = vb_project::open_ron(ron).expect("drumkit loads");
    assert_eq!(
        doc.nodes
            .iter()
            .filter(|e| e.node.kind == "key-gate")
            .count(),
        3,
        "kick, snare, hat"
    );

    // Peak over a short window with the given notes held.
    let peak = |notes: &[u8]| {
        let ctx = BuildContext::default();
        for &n in notes {
            ctx.poly.note_on(n, 110);
        }
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256).expect("builds");
        let sink = sink.expect("has a sink");
        let mut p = 0.0_f32;
        for _ in 0..6 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 256,
            });
            let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
            assert!(out.iter().all(|s| s.is_finite()), "finite audio");
            p = p.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
        p
    };

    assert!(peak(&[]) < 1e-6, "silent at rest");
    assert!(
        peak(&[40]) < 1e-6,
        "a note with no drum mapped stays silent"
    );
    let kick = peak(&[36]);
    let hat = peak(&[42]);
    assert!(kick > 0.02, "kick sounds on note 36: {kick}");
    assert!(hat > 0.02, "hat sounds on note 42: {hat}");
    // Kick + hat held together: both voices sound (polyphonic), louder than the kick alone.
    let together = peak(&[36, 42]);
    assert!(
        together > kick,
        "simultaneous drums sum: kick {kick} vs kick+hat {together}"
    );
}

/// Peak absolute output over `blocks` blocks of `frames`, for a patch built with `ctx`.
fn peak_over(ron: &str, ctx: &BuildContext, frames: usize, blocks: usize) -> f32 {
    let registry = Registry::with_builtins();
    let doc = vb_project::open_ron(ron).expect("loads");
    let (mut engine, sink) = engine_from_doc(&doc, &registry, ctx, frames).expect("builds");
    let sink = sink.expect("has a sink");
    let mut p = 0.0_f32;
    for _ in 0..blocks {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames,
        });
        let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
        for s in out {
            assert!(s.is_finite(), "non-finite sample");
            p = p.max(s.abs());
        }
    }
    p
}

#[test]
fn supersaw_lead_is_silent_at_rest_and_fat_on_a_note() {
    let ron = include_str!("../../../examples/supersaw_lead.ron");
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "supersaw lead must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(52, 110); // ~165 Hz
    assert!(
        peak_over(ron, &ctx, 256, 8) > 0.01,
        "supersaw lead must sound on a note"
    );
}

#[test]
fn fm_ep_is_silent_at_rest_and_rings_on_a_note() {
    let ron = include_str!("../../../examples/fm_ep.ron");
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "FM EP must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(60, 110); // middle C
    assert!(
        peak_over(ron, &ctx, 256, 8) > 0.01,
        "FM EP must ring out on a note"
    );
}

#[test]
fn wavetable_pad_is_a_wavetable_voice_silent_at_rest_and_blooms_on_a_note() {
    let ron = include_str!("../../../examples/wavetable_pad.ron");

    // It actually exercises the `wavetable` node (the point of this patch): two oscillators.
    let doc = vb_project::open_ron(ron).expect("wavetable_pad loads");
    let wavetables = doc
        .nodes
        .iter()
        .filter(|e| e.node.kind == "wavetable")
        .count();
    assert_eq!(wavetables, 2, "a two-oscillator wavetable voice");

    // Silent until a note (VCA biased shut; reverb of silence is silence).
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "wavetable pad must be silent until a note plays"
    );

    // A held note blooms in over the slow pad attack (~0.4 s ≈ 75 blocks) — give it time.
    let ctx = BuildContext::default();
    ctx.notes.note_on(55, 100); // ~196 Hz
    assert!(
        peak_over(ron, &ctx, 256, 150) > 0.01,
        "wavetable pad must sing once the slow envelope opens"
    );
}

#[test]
fn pluck_string_is_a_karplus_voice_silent_until_plucked() {
    let ron = include_str!("../../../examples/pluck_string.ron");

    // It exercises the `karplus` physical-model node.
    let doc = vb_project::open_ron(ron).expect("pluck_string loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "karplus"),
        "the plucked-string voice is built on karplus"
    );

    // No gate has fired → the string is at rest → silence.
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "an unplucked string is silent"
    );

    // A note fires the gate → the pluck rings out.
    let ctx = BuildContext::default();
    ctx.notes.note_on(57, 110); // ~220 Hz
    assert!(
        peak_over(ron, &ctx, 256, 16) > 0.01,
        "a plucked string must ring"
    );
}

#[test]
fn sync_lead_is_a_hard_sync_voice_silent_at_rest_and_bright_on_a_note() {
    let ron = include_str!("../../../examples/sync_lead.ron");

    let doc = vb_project::open_ron(ron).expect("sync_lead loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "hard-sync"),
        "the sync lead is built on hard-sync"
    );

    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "sync lead must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(48, 110); // ~130 Hz
    assert!(
        peak_over(ron, &ctx, 256, 16) > 0.01,
        "sync lead must sound on a note"
    );
}

#[test]
fn vowel_pad_is_a_formant_voice_silent_at_rest_and_speaks_on_a_note() {
    let ron = include_str!("../../../examples/vowel_pad.ron");

    let doc = vb_project::open_ron(ron).expect("vowel_pad loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "formant"),
        "the vowel pad is built on the formant filter"
    );

    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "vowel pad must be silent until a note plays"
    );
    // Pad attack ~0.3 s ≈ 56 blocks — give it time to open.
    let ctx = BuildContext::default();
    ctx.notes.note_on(50, 100); // ~147 Hz
    assert!(
        peak_over(ron, &ctx, 256, 120) > 0.01,
        "vowel pad must speak once the envelope opens"
    );
}

#[test]
fn granular_cloud_is_a_granular_voice_silent_at_rest_and_clouds_on_a_note() {
    let ron = include_str!("../../../examples/granular_cloud.ron");
    let doc = vb_project::open_ron(ron).expect("granular_cloud loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "granular"),
        "the cloud is built on the granular node"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "granular cloud must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(50, 100);
    assert!(
        peak_over(ron, &ctx, 256, 120) > 0.01,
        "granular cloud must sound once the envelope opens"
    );
}

#[test]
fn west_coast_fold_is_a_wavefolder_voice_silent_at_rest_and_rich_on_a_note() {
    let ron = include_str!("../../../examples/west_coast_fold.ron");
    let doc = vb_project::open_ron(ron).expect("west_coast_fold loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "wavefolder"),
        "the voice is built on the wavefolder"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "west-coast fold must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(45, 110);
    assert!(
        peak_over(ron, &ctx, 256, 16) > 0.01,
        "west-coast fold must sound on a note"
    );
}

#[test]
fn ring_bell_is_a_ring_mod_voice_silent_at_rest_and_chimes_on_a_note() {
    let ron = include_str!("../../../examples/ring_bell.ron");
    let doc = vb_project::open_ron(ron).expect("ring_bell loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "ring-mod"),
        "the bell is built on the ring modulator"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "ring bell must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(60, 120);
    assert!(
        peak_over(ron, &ctx, 256, 16) > 0.01,
        "ring bell must chime on a note"
    );
}

#[test]
fn resonator_pluck_is_a_resonator_voice_silent_at_rest_and_rings_on_a_note() {
    let ron = include_str!("../../../examples/resonator_pluck.ron");
    let doc = vb_project::open_ron(ron).expect("resonator_pluck loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "resonator"),
        "the voice is built on the resonator"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "struck resonator must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(55, 120);
    assert!(
        peak_over(ron, &ctx, 256, 24) > 0.01,
        "a struck resonator must ring"
    );
}

#[test]
fn lofi_crush_is_a_bitcrusher_voice_silent_at_rest_and_crunchy_on_a_note() {
    let ron = include_str!("../../../examples/lofi_crush.ron");
    let doc = vb_project::open_ron(ron).expect("lofi_crush loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "bitcrusher"),
        "the voice is built on the bitcrusher"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "lo-fi crush must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(48, 110);
    assert!(
        peak_over(ron, &ctx, 256, 16) > 0.01,
        "lo-fi crush must sound on a note"
    );
}

#[test]
fn random_steps_is_a_sample_hold_voice_silent_at_rest_and_burbles_on_a_note() {
    let ron = include_str!("../../../examples/random_steps.ron");
    let doc = vb_project::open_ron(ron).expect("random_steps loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "sample-hold"),
        "the stepped filter is driven by sample-hold"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "random steps must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(45, 110);
    assert!(
        peak_over(ron, &ctx, 256, 32) > 0.01,
        "random steps must sound on a note"
    );
}

#[test]
fn flanged_sweep_is_a_flanger_voice_silent_at_rest_and_sweeps_on_a_note() {
    let ron = include_str!("../../../examples/flanged_sweep.ron");
    let doc = vb_project::open_ron(ron).expect("flanged_sweep loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "flanger"),
        "the voice is built on the flanger node"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "flanged sweep must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(45, 110); // ~110 Hz
    assert!(
        peak_over(ron, &ctx, 256, 16) > 0.01,
        "flanged sweep must sound on a note"
    );
}

#[test]
fn metallic_shift_is_a_freq_shifter_voice_silent_at_rest_and_rings_on_a_note() {
    let ron = include_str!("../../../examples/metallic_shift.ron");
    let doc = vb_project::open_ron(ron).expect("metallic_shift loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "freq-shifter"),
        "the inharmonic voice is built on the frequency shifter"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "metallic shift must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(57, 110); // ~220 Hz
    assert!(
        peak_over(ron, &ctx, 256, 16) > 0.01,
        "metallic shift must sound on a note"
    );
}

#[test]
fn octave_stack_is_a_pitch_shifter_voice_silent_at_rest_and_thick_on_a_note() {
    let ron = include_str!("../../../examples/octave_stack.ron");
    let doc = vb_project::open_ron(ron).expect("octave_stack loads");
    assert!(
        doc.nodes.iter().any(|e| e.node.kind == "pitch-shifter"),
        "the octave-stacked voice is built on the pitch shifter"
    );
    assert!(
        peak_over(ron, &BuildContext::default(), 256, 8) < 1e-6,
        "octave stack must be silent until a note plays"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(45, 110); // ~110 Hz saw → its +1-octave blend
    assert!(
        // The pitch shifter's STFT latency (~768 frames) plus a frame means the voice needs a
        // few blocks before the shifted layer rings out; 24 × 256 frames clears it.
        peak_over(ron, &ctx, 256, 24) > 0.01,
        "octave stack must sound on a note"
    );
}

#[test]
fn auto_wah_drones_and_stays_bounded() {
    // The auto-wah patch has a built-in saw drone (no note needed), tremolo-driven through
    // an envelope-follower-swept filter. It should produce sustained, bounded audio.
    let ron = include_str!("../../../examples/auto_wah.ron");
    let peak = peak_over(ron, &BuildContext::default(), 256, 16);
    assert!(
        peak > 0.01,
        "auto-wah drone must be audible without a note: {peak}"
    );
    assert!(peak <= 4.0, "auto-wah output stays bounded: {peak}");
}

#[test]
fn the_example_set_is_the_expected_manifest() {
    // A guard so the committed set and this manifest stay in step.
    let names: Vec<&str> = EXAMPLES.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        vec![
            "synth",
            "delay",
            "unison",
            "tremolo",
            "filter_sweep",
            "vibrato",
            "first_synth",
            "ladder_bass",
            "mono_lead",
            "minimoog",
            "minitaur",
            "neutron",
            "edge",
            "tb303",
            "sh101",
            "juno60",
            "grandmother",
            "maul",
            "minilogue",
            "drumkit",
            "supersaw_lead",
            "fm_ep",
            "auto_wah",
            "super_bass",
            "organ_phaser",
            "stair_glide",
            "clock_poly",
            "euclid_pulse",
            "step_lead",
            "auto_pan",
            "stereo_lead",
            "pan_perc",
            "wavetable_pad",
            "pluck_string",
            "sync_lead",
            "vowel_pad",
            "granular_cloud",
            "west_coast_fold",
            "ring_bell",
            "resonator_pluck",
            "lofi_crush",
            "random_steps",
            "flanged_sweep",
            "metallic_shift",
            "octave_stack"
        ]
    );
    // No example is empty.
    assert!(EXAMPLES.iter().all(|(_, ron)| !ron.trim().is_empty()));
}

/// Peak absolute output over `blocks` processed frames of `ron`, in the given context.
fn showcase_peak(ron: &str, ctx: &BuildContext, blocks: usize) -> f32 {
    let registry = Registry::with_builtins();
    let doc = vb_project::open_ron(ron).expect("showcase loads");
    let (mut engine, sink) = engine_from_doc(&doc, &registry, ctx, 256).expect("builds");
    let sink = sink.expect("has a sink");
    let mut peak = 0.0_f32;
    for _ in 0..blocks {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
        assert!(out.iter().all(|s| s.is_finite()), "non-finite output");
        peak = peak.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
    }
    peak
}

#[test]
fn super_bass_is_silent_at_rest_and_sounds_on_a_note() {
    // pitch-osc + sub-osc → compressor, gated by a note (VCA bias 0 → silent at rest).
    let ron = include_str!("../../../examples/super_bass.ron");
    assert!(
        showcase_peak(ron, &BuildContext::default(), 8) < 1e-6,
        "super_bass must be silent at rest"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(33, 100); // ~55 Hz
    assert!(
        showcase_peak(ron, &ctx, 8) > 0.01,
        "super_bass must sound on a note"
    );
}

#[test]
fn organ_phaser_is_silent_at_rest_and_sounds_on_a_note() {
    // additive organ → phaser, gated by a note.
    let ron = include_str!("../../../examples/organ_phaser.ron");
    assert!(
        showcase_peak(ron, &BuildContext::default(), 8) < 1e-6,
        "organ_phaser must be silent at rest"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(57, 100); // ~220 Hz
    assert!(
        showcase_peak(ron, &ctx, 8) > 0.01,
        "organ_phaser must sound on a note"
    );
}

#[test]
fn stair_glide_is_silent_at_rest_and_sounds_on_a_note() {
    // glide → quantizer → pitch-osc, gated by a note.
    let ron = include_str!("../../../examples/stair_glide.ron");
    assert!(
        showcase_peak(ron, &BuildContext::default(), 8) < 1e-6,
        "stair_glide must be silent at rest"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(60, 100); // middle C
    assert!(
        showcase_peak(ron, &ctx, 8) > 0.01,
        "stair_glide must sound on a note"
    );
}

#[test]
fn clock_poly_plays_itself_without_a_note() {
    // clock + clock-divider gate two voices — a self-playing polyrhythm, audible at rest.
    let ron = include_str!("../../../examples/clock_poly.ron");
    // A few seconds of blocks so several clock beats (and a divided beat) land.
    let peak = showcase_peak(ron, &BuildContext::default(), 400);
    assert!(peak > 0.01, "clock_poly plays itself: peak {peak}");
    assert!(peak <= 4.0, "clock_poly output stays bounded: {peak}");
}

#[test]
fn euclid_pulse_plays_itself_as_a_euclidean_groove() {
    // clock → euclid → ahr gates a noise burst through gate → dc-blocker → limiter.
    // Self-playing (the clock lives inside), so it is audible at rest, no note needed.
    let ron = include_str!("../../../examples/euclid_pulse.ron");
    let peak = showcase_peak(ron, &BuildContext::default(), 600);
    assert!(peak > 0.01, "euclid_pulse plays itself: peak {peak}");
    assert!(
        peak <= 1.5,
        "euclid_pulse stays under the limiter ceiling: {peak}"
    );
}

#[test]
fn step_lead_is_silent_at_rest_and_sounds_on_a_note() {
    // clock → step-sequencer → vco → tone, note-gated by an AR envelope through a
    // bias-0 VCA: silent until a note opens the envelope, then a stepped sequenced lead.
    let ron = include_str!("../../../examples/step_lead.ron");
    assert!(
        showcase_peak(ron, &BuildContext::default(), 8) < 1e-6,
        "step_lead must be silent at rest"
    );
    let ctx = BuildContext::default();
    ctx.notes.note_on(60, 100); // gate only — pitch comes from the sequencer
    assert!(
        showcase_peak(ron, &ctx, 16) > 0.01,
        "step_lead must sound on a note"
    );
}

#[test]
fn auto_pan_produces_a_moving_stereo_image() {
    // oscillator → pan ← lfo: the pan node is a two-channel (stereo) sink, and the LFO
    // sweeps the image so the left and right channels diverge over time.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/auto_pan.ron");
    let doc = vb_project::open_ron(ron).expect("auto_pan loads");
    let (mut engine, sink) =
        engine_from_doc(&doc, &registry, &BuildContext::default(), 256).expect("builds");
    let sink = sink.expect("has a sink");

    let mut max_diff = 0.0_f32;
    let mut both_carry_signal = false;
    // ~1 s of audio: enough for the 2 Hz LFO to sweep the field.
    for _ in 0..200 {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let buf = engine.node_output(sink).expect("sink output");
        assert_eq!(buf.channels(), 2, "the pan sink is stereo");
        let l = buf.channel(0).unwrap();
        let r = buf.channel(1).unwrap();
        assert!(l.iter().chain(r.iter()).all(|s| s.is_finite()));
        for (a, b) in l.iter().zip(r.iter()) {
            max_diff = max_diff.max((a - b).abs());
        }
        if l.iter().any(|s| s.abs() > 0.05) && r.iter().any(|s| s.abs() > 0.05) {
            both_carry_signal = true;
        }
    }
    assert!(both_carry_signal, "both channels carry signal as it pans");
    assert!(
        max_diff > 0.1,
        "the stereo image moves (L differs from R): {max_diff}"
    );
}

/// `(peak over channel 0, sink channel count, max |L-R| seen)` for a showcase patch.
fn stereo_stats(ron: &str, ctx: &BuildContext, blocks: usize) -> (f32, usize, f32) {
    let registry = Registry::with_builtins();
    let doc = vb_project::open_ron(ron).expect("showcase loads");
    let (mut engine, sink) = engine_from_doc(&doc, &registry, ctx, 256).expect("builds");
    let sink = sink.expect("has a sink");
    let (mut peak, mut channels, mut max_diff) = (0.0_f32, 0usize, 0.0_f32);
    for _ in 0..blocks {
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let buf = engine.node_output(sink).expect("sink output");
        channels = buf.channels();
        let l = buf.channel(0).unwrap();
        peak = peak.max(l.iter().fold(0.0, |m, s| m.max(s.abs())));
        if let Some(r) = buf.channel(1) {
            for (a, b) in l.iter().zip(r.iter()) {
                max_diff = max_diff.max((a - b).abs());
            }
        }
    }
    (peak, channels, max_diff)
}

#[test]
fn stereo_lead_is_silent_at_rest_and_plays_a_stereo_note() {
    // midi → pitch-osc → adsr·vca → ping-pong → width: a note-gated lead that ends stereo.
    let ron = include_str!("../../../examples/stereo_lead.ron");
    let (at_rest, _, _) = stereo_stats(ron, &BuildContext::default(), 8);
    assert!(
        at_rest < 1e-6,
        "stereo_lead must be silent at rest: {at_rest}"
    );

    let ctx = BuildContext::default();
    ctx.notes.note_on(57, 100); // ~220 Hz
                                // Enough blocks (~0.5 s) for the short ping-pong echoes to populate both channels.
    let (peak, channels, diff) = stereo_stats(ron, &ctx, 90);
    assert!(peak > 0.01, "stereo_lead must sound on a note: {peak}");
    assert_eq!(channels, 2, "the width node makes it a stereo sink");
    assert!(
        diff > 0.05,
        "ping-pong + width give a real stereo image: {diff}"
    );
}

#[test]
fn pan_perc_plays_itself_in_stereo() {
    // clock → euclid → ahr gates noise through a vca into an LFO-swept pan: a self-playing
    // Euclidean percussion that auto-pans across a true two-channel sink.
    let ron = include_str!("../../../examples/pan_perc.ron");
    let (peak, channels, diff) = stereo_stats(ron, &BuildContext::default(), 600);
    assert!(peak > 0.01, "pan_perc plays itself: {peak}");
    assert_eq!(channels, 2, "the pan node makes it a stereo sink");
    assert!(
        diff > 0.1,
        "the percussion moves across the stereo field: {diff}"
    );
    assert!(peak <= 1.5, "output stays bounded: {peak}");
}
