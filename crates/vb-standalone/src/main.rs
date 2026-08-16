//! `vb-standalone` — hear the graph, and hear it change.
//!
//! Compiles a patch into the allocation-free [`vb_runtime::RtEngine`], streams it to
//! the default output device via cpal, and cycles through patch variants *while the
//! stream runs*: each variant is compiled on the main thread and handed to the audio
//! callback through the lock-free engine swap, so the sound changes with no dropout.
//! The egui editor (Epic 4) will drive the same [`vb_runtime::EngineSender`].
//!
//! Behaviour: press Enter to switch to the next patch, `q`+Enter to quit. Set
//! `VB_DEMO_SECS=<n>` to auto-cycle through all variants over `n` seconds and exit
//! (useful for scripts).

mod audio;

use std::collections::HashMap;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use vb_core::{Graph, GraphNode, NodeId, Port, PortId, Processor, SignalType};
use vb_nodes::{Biquad, FilterKind, Gain, Oscillator, Waveform};
use vb_runtime::{engine_swap, RtEngine};

use audio::BlockFeeder;

/// Engine block size in frames. The feeder bridges this to whatever the device asks.
const BLOCK_FRAMES: usize = 256;

/// A patch variant the demo can switch to while playing.
struct Variant {
    name: &'static str,
    waveform: Waveform,
    freq_hz: f32,
    cutoff_hz: f32,
    q: f32,
    gain: f32,
}

const VARIANTS: &[Variant] = &[
    Variant {
        name: "warm saw",
        waveform: Waveform::Saw,
        freq_hz: 110.0,
        cutoff_hz: 1_200.0,
        q: 0.707,
        gain: 0.2,
    },
    Variant {
        name: "hollow square",
        waveform: Waveform::Square,
        freq_hz: 220.0,
        cutoff_hz: 600.0,
        q: 2.0,
        gain: 0.12,
    },
    Variant {
        name: "acid bass",
        waveform: Waveform::Saw,
        freq_hz: 55.0,
        cutoff_hz: 300.0,
        q: 4.0,
        gain: 0.18,
    },
    Variant {
        name: "pure sine",
        waveform: Waveform::Sine,
        freq_hz: 440.0,
        cutoff_hz: 18_000.0,
        q: 0.707,
        gain: 0.2,
    },
];

/// Builds `osc → filter → gain` for a variant.
///
/// Every variant uses the same three-node shape in the same insertion order, so the
/// sink (the gain node) always gets the same [`NodeId`] — the feeder keeps playing the
/// same id across swaps.
fn build_variant(v: &Variant, frames: usize) -> (RtEngine, NodeId) {
    let mut g = Graph::new();
    let osc = g.add_node(GraphNode::new(
        "oscillator",
        vec![],
        vec![Port::output("out", SignalType::Audio)],
    ));
    let flt = g.add_node(GraphNode::new(
        "biquad",
        vec![Port::input("in", SignalType::Audio)],
        vec![Port::output("out", SignalType::Audio)],
    ));
    let amp = g.add_node(GraphNode::new(
        "gain",
        vec![Port::input("in", SignalType::Audio)],
        vec![Port::output("out", SignalType::Audio)],
    ));
    g.connect(osc, PortId(0), flt, PortId(0))
        .expect("osc→filter");
    g.connect(flt, PortId(0), amp, PortId(0))
        .expect("filter→gain");

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(osc, Box::new(Oscillator::new(v.waveform, v.freq_hz)));
    procs.insert(
        flt,
        Box::new(Biquad::new(FilterKind::LowPass, v.cutoff_hz, v.q)),
    );
    procs.insert(amp, Box::new(Gain::new(v.gain)));

    let engine = RtEngine::compile(&g, procs, frames).expect("variant compiles");
    (engine, amp)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default audio output device")?;
    let config = device.default_output_config()?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("unsupported sample format {:?}", config.sample_format()).into());
    }

    let sample_rate = config.sample_rate() as f32;
    let channels = config.channels() as usize;

    let (tx, rx) = engine_swap();
    let (engine, sink) = build_variant(&VARIANTS[0], BLOCK_FRAMES);
    let mut feeder = BlockFeeder::new(engine, Some(rx), sink, sample_rate);

    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unknown".into());
    println!("vb-standalone on '{device_name}' @ {sample_rate} Hz, {channels} ch");
    println!("▶ {}", VARIANTS[0].name);

    let stream = device.build_output_stream(
        config.into(),
        move |data: &mut [f32], _info| feeder.fill(data, channels),
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?;
    stream.play()?;

    let demo_secs = std::env::var("VB_DEMO_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    match demo_secs {
        Some(secs) => {
            // Scripted: hold each variant for an equal share of the requested time.
            let hold = std::time::Duration::from_millis(secs * 1000 / VARIANTS.len() as u64);
            std::thread::sleep(hold);
            for v in &VARIANTS[1..] {
                let (engine, _) = build_variant(v, BLOCK_FRAMES);
                tx.install(engine);
                println!("▶ {}", v.name);
                std::thread::sleep(hold);
                tx.collect();
            }
        }
        None => {
            println!("Enter — next patch, q+Enter — quit");
            let mut next = 1_usize;
            loop {
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line).is_err() || line.trim() == "q" {
                    break;
                }
                let v = &VARIANTS[next % VARIANTS.len()];
                let (engine, _) = build_variant(v, BLOCK_FRAMES);
                tx.install(engine);
                println!("▶ {}", v.name);
                next += 1;
                tx.collect();
            }
        }
    }
    Ok(())
}
