//! `vb-standalone` — hear the graph.
//!
//! Builds a small demo patch (saw oscillator → low-pass filter → gain), compiles it
//! into the allocation-free [`vb_runtime::RtEngine`] and streams it to the default
//! output device via cpal. The editor UI arrives in Epic 4; for now this binary is the
//! proof that the whole pipeline — graph, nodes, RT engine, device output — makes
//! sound.
//!
//! Behaviour: plays until Enter is pressed. Set `VB_DEMO_SECS=<n>` to play for `n`
//! seconds and exit instead (useful for scripts).

mod audio;

use std::collections::HashMap;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use vb_core::{Graph, GraphNode, NodeId, Port, Processor, SignalType};
use vb_nodes::{Biquad, FilterKind, Gain, Oscillator, Waveform};
use vb_runtime::RtEngine;

use audio::BlockFeeder;

/// Engine block size in frames. The feeder bridges this to whatever the device asks.
const BLOCK_FRAMES: usize = 256;

/// Demo patch: saw 110 Hz → low-pass 1.2 kHz → gain 0.2. Returns the engine and the
/// node to play.
fn demo_patch(frames: usize) -> (RtEngine, NodeId) {
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
    g.connect(osc, vb_core::PortId(0), flt, vb_core::PortId(0))
        .expect("osc→filter");
    g.connect(flt, vb_core::PortId(0), amp, vb_core::PortId(0))
        .expect("filter→gain");

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(osc, Box::new(Oscillator::new(Waveform::Saw, 110.0)));
    procs.insert(
        flt,
        Box::new(Biquad::new(FilterKind::LowPass, 1_200.0, 0.707)),
    );
    procs.insert(amp, Box::new(Gain::new(0.2)));

    let engine = RtEngine::compile(&g, procs, frames).expect("demo patch compiles");
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
    let (engine, sink) = demo_patch(BLOCK_FRAMES);
    let mut feeder = BlockFeeder::new(engine, None, sink, sample_rate);

    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unknown".into());
    println!(
        "vb-standalone: saw→LP→gain demo on '{device_name}' @ {sample_rate} Hz, {channels} ch"
    );

    let stream = device.build_output_stream(
        config.into(),
        move |data: &mut [f32], _info| feeder.fill(data, channels),
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?;
    stream.play()?;

    match std::env::var("VB_DEMO_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(secs) => {
            std::thread::sleep(std::time::Duration::from_secs(secs));
        }
        None => {
            println!("playing — press Enter to quit");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
        }
    }
    Ok(())
}
