//! `seclab-standalone` — the live editor: patch on the canvas, sound in your ears.
//!
//! Opens the egui node editor with a cpal stream running behind it. Every audible
//! edit (add/remove/wire/param/undo) recompiles the patch into a fresh
//! allocation-free [`seclab_runtime::RtEngine`] and hands it to the audio callback
//! through the lock-free engine swap — the sound follows the canvas with no
//! dropouts. This closes the editor side of task 000062.
//!
//! `VB_DEMO_SECS=<n>` keeps the old headless mode: auto-cycle four preset patch
//! variants over `n` seconds and exit (useful for scripts; no window).

mod audio;
mod live;
mod midi_io;
mod scope;
mod settings;

use std::collections::HashMap;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use seclab_core::{Graph, GraphNode, NodeId, Port, PortId, Processor, SignalType};
use seclab_editor::EditorApp;
use seclab_nodes::{Biquad, BuildContext, FilterKind, Gain, Oscillator, Registry, Waveform};
use seclab_runtime::{engine_swap, EngineSender, RtEngine};

use audio::{BlockFeeder, LoadMonitor, SinkHandle};
use live::build_live_engine;
use midi_io::{connect_port, list_ports};
use scope::{peak, rms, spectrum, Scope, SCOPE_LEN, SPECTRUM_BANDS};
use settings::{rates_within, BLOCK_OPTIONS};

/// Engine block size in frames. The feeder bridges this to whatever the device asks.
const BLOCK_FRAMES: usize = 256;

/// One running audio stream and its engine-swap channel.
struct AudioStack {
    tx: EngineSender,
    sink: SinkHandle,
    /// DSP load / xrun counters published by the audio thread.
    monitor: LoadMonitor,
    /// Visualization tap of the played signal.
    scope: Scope,
    /// Keeps the cpal stream alive for the stack's lifetime.
    _stream: cpal::Stream,
}

/// Starts a stream on `device` at the requested rate and engine block size.
///
/// The engine block size is independent of the device buffer (the feeder bridges
/// them), so any [`BLOCK_OPTIONS`] entry works on any device.
fn start_stack(
    device: &cpal::Device,
    sample_rate: u32,
    block_frames: usize,
    editor: &EditorApp,
    registry: &Registry,
    build_ctx: &BuildContext,
) -> Result<AudioStack, String> {
    let default = device.default_output_config().map_err(|e| e.to_string())?;
    if default.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!(
            "unsupported sample format {:?}",
            default.sample_format()
        ));
    }
    let channels = default.channels() as usize;

    let (engine, sink_id) = build_live_engine(editor.patch(), registry, build_ctx, block_frames)
        .ok_or("the patch failed to compile")?;
    let (tx, rx) = engine_swap();
    let sink = SinkHandle::new(sink_id);
    let monitor = LoadMonitor::new();
    let scope = Scope::new();
    let mut feeder = BlockFeeder::new(engine, Some(rx), sink.clone(), sample_rate as f32)
        .with_monitor(monitor.clone())
        .with_scope(scope.clone());

    let config = cpal::StreamConfig {
        channels: default.channels(),
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _info| feeder.fill(data, channels),
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(AudioStack {
        tx,
        sink,
        monitor,
        scope,
        _stream: stream,
    })
}

/// Human-readable device name, best effort.
fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unknown device".into())
}

/// Sample rates worth offering for `device`: common rates its f32 configs cover,
/// plus its default rate.
fn supported_rates(device: &cpal::Device) -> Vec<u32> {
    let ranges: Vec<(u32, u32)> = device
        .supported_output_configs()
        .map(|configs| {
            configs
                .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
                .map(|c| (c.min_sample_rate(), c.max_sample_rate()))
                .collect()
        })
        .unwrap_or_default();
    let mut rates = rates_within(&ranges);
    if let Ok(default) = device.default_output_config() {
        if !rates.contains(&default.sample_rate()) {
            rates.push(default.sample_rate());
            rates.sort_unstable();
        }
    }
    rates
}

/// The editor window with the audio stream that follows it.
struct LiveApp {
    editor: EditorApp,
    registry: Registry,
    /// Live inputs handed to every engine rebuild (`midi-in` nodes listen to
    /// `build_ctx.notes`; the midir wiring lands with task 000061).
    build_ctx: BuildContext,
    /// Output devices as enumerated at startup, parallel to `device_names`.
    devices: Vec<cpal::Device>,
    device_names: Vec<String>,
    /// Selection shown in the audio window (applied on "apply").
    sel_device: usize,
    sel_rate: u32,
    sel_block: usize,
    /// Rates offered for the selected device.
    rates: Vec<u32>,
    stack: AudioStack,
    /// Last patch revision compiled into the running engine.
    compiled_revision: u64,
    /// Why the last apply failed, if it did.
    audio_error: Option<String>,
    /// MIDI input ports as last enumerated.
    midi_ports: Vec<String>,
    /// Index of the connected port in `midi_ports`.
    sel_midi: Option<usize>,
    /// The live MIDI connection; dropping it disconnects.
    midi_conn: Option<midir::MidiInputConnection<()>>,
    /// Why the last MIDI connect failed, if it did.
    midi_error: Option<String>,
    /// Reusable snapshot buffer for the visualization (UI thread only).
    scope_buf: Box<[f32; SCOPE_LEN]>,
    /// Whether the visualization panel is shown.
    show_scope: bool,
}

impl LiveApp {
    /// Renders the floating audio-settings window and applies changes.
    fn audio_settings_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("audio")
            .default_open(false)
            .resizable(false)
            .show(ctx, |ui| {
                let mut device_changed = false;
                egui::ComboBox::from_label("device")
                    .selected_text(
                        self.device_names
                            .get(self.sel_device)
                            .map_or("?", String::as_str),
                    )
                    .show_ui(ui, |ui| {
                        for (i, name) in self.device_names.iter().enumerate() {
                            if ui.selectable_value(&mut self.sel_device, i, name).changed() {
                                device_changed = true;
                            }
                        }
                    });
                if device_changed {
                    if let Some(device) = self.devices.get(self.sel_device) {
                        self.rates = supported_rates(device);
                        if !self.rates.contains(&self.sel_rate) {
                            self.sel_rate = self.rates.first().copied().unwrap_or(48_000);
                        }
                    }
                }

                egui::ComboBox::from_label("sample rate")
                    .selected_text(format!("{} Hz", self.sel_rate))
                    .show_ui(ui, |ui| {
                        for rate in &self.rates {
                            ui.selectable_value(&mut self.sel_rate, *rate, format!("{rate} Hz"));
                        }
                    });

                egui::ComboBox::from_label("block size")
                    .selected_text(format!("{} frames", self.sel_block))
                    .show_ui(ui, |ui| {
                        for block in BLOCK_OPTIONS {
                            ui.selectable_value(
                                &mut self.sel_block,
                                *block,
                                format!("{block} frames"),
                            );
                        }
                    });

                if ui.button("apply").clicked() {
                    self.apply_audio_settings();
                }
                if let Some(err) = &self.audio_error {
                    ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x48), err);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("MIDI");
                    if ui.button("rescan").clicked() {
                        self.midi_ports = list_ports();
                        // The connected index may now point at a different port.
                        self.disconnect_midi();
                    }
                });
                if self.midi_ports.is_empty() {
                    ui.weak("no MIDI inputs");
                } else {
                    let mut clicked: Option<usize> = None;
                    for (i, name) in self.midi_ports.iter().enumerate() {
                        let connected = self.sel_midi == Some(i);
                        if ui.selectable_label(connected, name).clicked() {
                            clicked = Some(i);
                        }
                    }
                    if let Some(i) = clicked {
                        if self.sel_midi == Some(i) {
                            self.disconnect_midi();
                        } else {
                            self.connect_midi(i);
                        }
                    }
                }
                if let Some(err) = &self.midi_error {
                    ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x48), err);
                }
            });
    }

    /// Connects MIDI port `i`, replacing any current connection.
    fn connect_midi(&mut self, i: usize) {
        self.disconnect_midi();
        match connect_port(i, self.build_ctx.clone()) {
            Ok(conn) => {
                self.midi_conn = Some(conn);
                self.sel_midi = Some(i);
                self.midi_error = None;
            }
            Err(err) => self.midi_error = Some(err),
        }
    }

    /// Drops the current MIDI connection and releases any held note.
    fn disconnect_midi(&mut self) {
        self.midi_conn = None;
        self.sel_midi = None;
        // The keyboard is gone: whatever note it held must not drone forever.
        let (gate, note, _) = self.build_ctx.notes.read();
        if gate {
            self.build_ctx.notes.note_off(note);
        }
    }

    /// Draws the peak meter, oscilloscope trace, and coarse spectrum from the last
    /// snapshot in `self.scope_buf`.
    fn scope_ui(&mut self, ui: &mut egui::Ui) {
        use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

        let samples: &[f32] = self.scope_buf.as_slice();
        let level = peak(samples);
        let level_rms = rms(samples);
        let bands = spectrum(samples, self.sel_rate as f32);

        ui.horizontal(|ui| {
            ui.label(format!("peak {level:.2}  rms {level_rms:.2}"));
        });

        let (resp, painter) =
            ui.allocate_painter(Vec2::new(ui.available_width(), 90.0), Sense::hover());
        let rect = resp.rect;
        painter.rect_filled(rect, 2.0, Color32::from_gray(18));

        // Split the panel: left third oscilloscope, right two-thirds spectrum bars.
        let split = rect.left() + rect.width() * 0.5;
        let scope_rect = Rect::from_min_max(rect.min, Pos2::new(split, rect.max.y));
        let spec_rect = Rect::from_min_max(Pos2::new(split, rect.min.y), rect.max);

        // Oscilloscope: the trace centred vertically, amplitude ±1 fills the height.
        let mid_y = scope_rect.center().y;
        let half_h = scope_rect.height() * 0.45;
        let stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x2d, 0xb8, 0xa8));
        let step = (samples.len() as f32 / scope_rect.width().max(1.0)).max(1.0);
        let mut prev: Option<Pos2> = None;
        let mut x = 0.0;
        while x < scope_rect.width() {
            let idx = (x * step) as usize;
            if idx >= samples.len() {
                break;
            }
            let p = Pos2::new(
                scope_rect.left() + x,
                mid_y - samples[idx].clamp(-1.0, 1.0) * half_h,
            );
            if let Some(prev) = prev {
                painter.line_segment([prev, p], stroke);
            }
            prev = Some(p);
            x += 1.0;
        }

        // Spectrum: one bar per band, height ∝ magnitude.
        let n = SPECTRUM_BANDS.len();
        let bar_w = spec_rect.width() / n as f32;
        for (i, &mag) in bands.iter().enumerate() {
            let h = mag.clamp(0.0, 1.0) * spec_rect.height();
            let x0 = spec_rect.left() + i as f32 * bar_w + 1.0;
            let bar = Rect::from_min_max(
                Pos2::new(x0, spec_rect.bottom() - h),
                Pos2::new(x0 + bar_w - 2.0, spec_rect.bottom()),
            );
            painter.rect_filled(bar, 0.0, Color32::from_rgb(0xd9, 0xa4, 0x2b));
        }
    }

    /// Tears down the running stream and starts one with the selected settings.
    /// On failure the old stream keeps playing and the error is shown.
    fn apply_audio_settings(&mut self) {
        let Some(device) = self.devices.get(self.sel_device) else {
            self.audio_error = Some("selected device disappeared".into());
            return;
        };
        match start_stack(
            device,
            self.sel_rate,
            self.sel_block,
            &self.editor,
            &self.registry,
            &self.build_ctx,
        ) {
            Ok(stack) => {
                // Dropping the old stack stops its stream; the new one is already live.
                self.stack = stack;
                self.compiled_revision = self.editor.patch().revision();
                self.audio_error = None;
            }
            Err(err) => self.audio_error = Some(err),
        }
    }
}

impl eframe::App for LiveApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Visualization panel above the status strip (both take from the bottom).
        if self.show_scope {
            self.stack.scope.snapshot(&mut self.scope_buf);
            egui::TopBottomPanel::bottom("scope")
                .resizable(false)
                .show(ctx, |ui| self.scope_ui(ui));
        }
        // Status strip so the editor gets the remaining space.
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let load = self.stack.monitor.load();
                let xruns = self.stack.monitor.xruns();
                ui.label(format!("dsp {:.0}%", load * 100.0));
                if load > 0.8 {
                    ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x48), "overload");
                }
                ui.separator();
                let xrun_text = format!("xruns {xruns}");
                if xruns > 0 {
                    ui.colored_label(egui::Color32::from_rgb(0xd9, 0xa4, 0x2b), xrun_text);
                } else {
                    ui.label(xrun_text);
                }
                ui.separator();
                ui.label(
                    self.device_names
                        .get(self.sel_device)
                        .map_or("?", String::as_str),
                );
                ui.label(format!(
                    "@ {} Hz / {} frames",
                    self.sel_rate, self.sel_block
                ));
                ui.separator();
                ui.checkbox(&mut self.show_scope, "scope");
            });
        });
        // Keep the meter moving even when the user is idle.
        ctx.request_repaint_after(std::time::Duration::from_millis(150));

        // The embedded editor draws its own top/side/central panels into the space the
        // bottom panels leave — one shared editor, native and (soon) in the plugin window.
        eframe::App::update(&mut self.editor, ctx, frame);
        self.audio_settings_ui(ctx);

        // An audible edit happened: recompile and swap. A patch that no longer
        // compiles (e.g. mid-edit) keeps the previous engine running.
        let revision = self.editor.patch().revision();
        if revision != self.compiled_revision {
            self.compiled_revision = revision;
            if let Some((engine, sink)) = build_live_engine(
                self.editor.patch(),
                &self.registry,
                &self.build_ctx,
                self.sel_block,
            ) {
                self.stack.sink.set(sink);
                self.stack.tx.install(engine);
            }
        }
        // Reclaim engines the audio thread has retired.
        self.stack.tx.collect();
    }
}

/// Opens the live editor window over a cpal stream.
fn run_live_editor() -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let devices: Vec<cpal::Device> = host
        .output_devices()
        .map(Iterator::collect)
        .unwrap_or_default();
    if devices.is_empty() {
        return Err("no audio output devices".into());
    }
    let device_names: Vec<String> = devices.iter().map(device_name).collect();

    // Start on the default device (matched by name) at its default rate.
    let default_name = host.default_output_device().map(|d| device_name(&d));
    let sel_device = default_name
        .and_then(|name| device_names.iter().position(|n| *n == name))
        .unwrap_or(0);
    let device = &devices[sel_device];
    let sel_rate = device.default_output_config()?.sample_rate();
    let rates = supported_rates(device);

    let mut editor = EditorApp::new();
    // Share the per-user modules folder with the plugin so a macro saved in either is browsable in
    // both (the default is a relative `modules`, tied to the launch directory).
    editor.set_modules_dir(seclab_editor::user_modules_dir());
    // Restore the assistant's persisted provider/model/endpoint/privacy choices (never the key —
    // that stays in the keychain) from `settings.ron` in that same folder. The user clicks
    // "apply / connect" in the Settings panel to wire the provider.
    editor.load_assistant_settings();
    let registry = Registry::with_builtins();
    let build_ctx = BuildContext::default();
    let stack = start_stack(
        device,
        sel_rate,
        BLOCK_FRAMES,
        &editor,
        &registry,
        &build_ctx,
    )?;
    let compiled_revision = editor.patch().revision();

    println!(
        "seclab-standalone editor on '{}' @ {sel_rate} Hz",
        device_names[sel_device]
    );

    let midi_ports = list_ports();
    let mut app = LiveApp {
        editor,
        registry,
        build_ctx,
        devices,
        device_names,
        sel_device,
        sel_rate,
        sel_block: BLOCK_FRAMES,
        rates,
        stack,
        compiled_revision,
        audio_error: None,
        midi_ports,
        sel_midi: None,
        midi_conn: None,
        midi_error: None,
        scope_buf: Box::new([0.0; SCOPE_LEN]),
        show_scope: true,
    };
    // Plug in the first keyboard found; the audio window can switch or disconnect.
    if !app.midi_ports.is_empty() {
        app.connect_midi(0);
        if let Some(port) = app.sel_midi.and_then(|i| app.midi_ports.get(i)) {
            println!("midi input: '{port}'");
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SecLab")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native("SecLab", options, Box::new(|_cc| Ok(Box::new(app))))?;
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
    println!("seclab-standalone demo on '{device_name}' @ {sample_rate} Hz, {channels} ch");
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
