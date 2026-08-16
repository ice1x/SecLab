//! `vb-standalone` — the live editor: patch on the canvas, sound in your ears.
//!
//! Opens the egui node editor with a cpal stream running behind it. Every audible
//! edit (add/remove/wire/param/undo) recompiles the patch into a fresh
//! allocation-free [`vb_runtime::RtEngine`] and hands it to the audio callback
//! through the lock-free engine swap — the sound follows the canvas with no
//! dropouts. This closes the editor side of task 000062.
//!
//! `VB_DEMO_SECS=<n>` keeps the old headless mode: auto-cycle four preset patch
//! variants over `n` seconds and exit (useful for scripts; no window).

mod audio;
mod live;

use std::collections::HashMap;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use vb_core::{Graph, GraphNode, NodeId, Port, PortId, Processor, SignalType};
use vb_editor::EditorApp;
use vb_nodes::{Biquad, FilterKind, Gain, Oscillator, Registry, Waveform};
use vb_runtime::{engine_swap, EngineSender, RtEngine};

use audio::{BlockFeeder, SinkHandle};
use live::build_live_engine;

/// Engine block size in frames. The feeder bridges this to whatever the device asks.
const BLOCK_FRAMES: usize = 256;

/// The editor window with the audio stream that follows it.
struct LiveApp {
    editor: EditorApp,
    registry: Registry,
    tx: EngineSender,
    sink: SinkHandle,
    /// Last patch revision compiled into the running engine.
    compiled_revision: u64,
    /// Keeps the cpal stream alive for the window's lifetime.
    _stream: cpal::Stream,
}

impl eframe::App for LiveApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.editor, ui, frame);

        // An audible edit happened: recompile and swap. A patch that no longer
        // compiles (e.g. mid-edit) keeps the previous engine running.
        let revision = self.editor.patch().revision();
        if revision != self.compiled_revision {
            self.compiled_revision = revision;
            if let Some((engine, sink)) =
                build_live_engine(self.editor.patch(), &self.registry, BLOCK_FRAMES)
            {
                self.sink.set(sink);
                self.tx.install(engine);
            }
        }
        // Reclaim engines the audio thread has retired.
        self.tx.collect();
    }
}

/// Opens the live editor window over a cpal stream.
fn run_live_editor() -> Result<(), Box<dyn std::error::Error>> {
    let (device, config) = default_output()?;
    let sample_rate = config.sample_rate() as f32;
    let channels = config.channels() as usize;

    let editor = EditorApp::new();
    let registry = Registry::with_builtins();
    let (engine, sink_id) = build_live_engine(editor.patch(), &registry, BLOCK_FRAMES)
        .ok_or("the starter patch failed to compile")?;
    let compiled_revision = editor.patch().revision();

    let (tx, rx) = engine_swap();
    let sink = SinkHandle::new(sink_id);
    let mut feeder = BlockFeeder::new(engine, Some(rx), sink.clone(), sample_rate);

    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unknown".into());
    println!("vb-standalone editor on '{device_name}' @ {sample_rate} Hz, {channels} ch");

    let stream = device.build_output_stream(
        config.into(),
        move |data: &mut [f32], _info| feeder.fill(data, channels),
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?;
    stream.play()?;

    let app = LiveApp {
        editor,
        registry,
        tx,
        sink,
        compiled_revision,
        _stream: stream,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VST_bomb")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native("VST_bomb", options, Box::new(|_cc| Ok(Box::new(app))))?;
    Ok(())
}

fn default_output(
) -> Result<(cpal::Device, cpal::SupportedStreamConfig), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default audio output device")?;
    let config = device.default_output_config()?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("unsupported sample format {:?}", config.sample_format()).into());
    }
    Ok((device, config))
}

// --- Legacy headless demo (VB_DEMO_SECS): cycle preset variants, no window. ---

/// A patch variant the headless demo can switch to while playing.
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

/// The pre-editor demo: hold each variant for an equal share of `secs`, then exit.
fn run_headless_demo(secs: u64) -> Result<(), Box<dyn std::error::Error>> {
    let (device, config) = default_output()?;
    let sample_rate = config.sample_rate() as f32;
    let channels = config.channels() as usize;

    let (tx, rx) = engine_swap();
    let (engine, sink) = build_variant(&VARIANTS[0], BLOCK_FRAMES);
    let mut feeder = BlockFeeder::new(engine, Some(rx), sink, sample_rate);

    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unknown".into());
    println!("vb-standalone demo on '{device_name}' @ {sample_rate} Hz, {channels} ch");
    println!("▶ {}", VARIANTS[0].name);

    let stream = device.build_output_stream(
        config.into(),
        move |data: &mut [f32], _info| feeder.fill(data, channels),
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?;
    stream.play()?;

    let hold = std::time::Duration::from_millis(secs * 1000 / VARIANTS.len() as u64);
    std::thread::sleep(hold);
    for v in &VARIANTS[1..] {
        let (engine, _) = build_variant(v, BLOCK_FRAMES);
        tx.install(engine);
        println!("▶ {}", v.name);
        std::thread::sleep(hold);
        tx.collect();
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::var("VB_DEMO_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(secs) => run_headless_demo(secs),
        None => run_live_editor(),
    }
}
