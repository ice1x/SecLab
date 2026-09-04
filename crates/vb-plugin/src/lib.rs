//! `vb-plugin` — the VST_bomb patch as a CLAP instrument (nih-plug).
//!
//! The scaffold (task 000064): a nih-plug plugin that loads a serialized project —
//! the path in the `VB_PATCH` environment variable, or the built-in default patch —
//! compiles it into the allocation-free [`vb_runtime::RtEngine`] at `initialize`,
//! and streams the patch's terminal audio node to the host in stereo. VST3/AUv2
//! arrive later via clap-wrapper (000069/000070); parameters, host MIDI, and the
//! embedded GUI are the next Epic 7 tasks.
//!
//! Build the CLAP with `cargo build -p vb-plugin --release`; the resulting
//! `libvb_plugin` cdylib renamed to `vb_bomb.clap` is a loadable CLAP bundle (the
//! one-command bundler is task 000072).

#[cfg(feature = "gui")]
pub mod editor;

// The instrument core now lives in the host-framework-free `vb-engine` crate (so it stays
// wasm32-portable for the browser audio path, backlog 000083). Re-exported under the old
// `engine_builder` name so existing `vb_plugin::engine_builder::…` paths keep resolving.
pub use vb_engine as engine_builder;

use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nih_plug::prelude::*;
use vb_core::NodeId;
use vb_nodes::{BuildContext, Registry};
use vb_project::ProjectDoc;
use vb_runtime::RtEngine;

use vb_runtime::{engine_swap, EngineReceiver};

pub use vb_engine::{default_patch, engine_from_doc, engine_from_doc_with_values, BuildError};

/// The VST_bomb instrument plugin.
pub struct VstBomb {
    params: Arc<VstBombParams>,
    /// The project this instance plays. Shared with the rebuild executor and
    /// replaceable when a state load restores a different patch.
    doc: Arc<Mutex<Arc<ProjectDoc>>>,
    registry: Arc<Registry>,
    build_ctx: BuildContext,
    /// Compiled in `initialize` (needs the host's buffer size), used in `process`.
    engine: Option<RtEngine>,
    sink: Option<NodeId>,
    sample_rate: f32,
    /// Block size the engine was compiled for, shared with the rebuild executor.
    frames: Arc<AtomicUsize>,
    /// Fresh engines from background rebuilds arrive here (lock-free).
    rx: EngineReceiver,
    /// The install side, handed to the background task executor once.
    tx: Option<vb_runtime::EngineSender>,
    /// Values the running engine was built with; compared in place, no allocation.
    built_values: Vec<f32>,
    /// Persisted egui window state for the embedded editor (000067).
    #[cfg(feature = "gui")]
    editor_state: Arc<nih_plug_egui::EguiState>,
}

/// One host-automatable parameter bound to a node's registry parameter.
struct BoundParam {
    node: NodeId,
    param: FloatParam,
    /// Stable host-facing identifier (`n<node>_<index>`).
    id: String,
}

/// The patch's parameters, one `FloatParam` per registry parameter of every node.
///
/// Built dynamically from the loaded document, so the host sees exactly the knobs
/// the patch has. IDs are stable across sessions (`n<node-id>_<param-index>`), which
/// also gives host preset state save/restore for free via nih-plug's serializer.
#[derive(Default)]
pub struct VstBombParams {
    bound: Vec<BoundParam>,
    /// The patch itself, serialized as RON — persisted inside the host's preset
    /// state so a saved DAW project carries the whole graph, not just knob values.
    patch_ron: Mutex<String>,
}

impl VstBombParams {
    /// The patch stored in this state, if it parses and validates.
    #[must_use]
    pub fn stored_patch(&self) -> Option<ProjectDoc> {
        let ron = self.patch_ron.lock().ok()?;
        if ron.is_empty() {
            return None;
        }
        vb_project::open_ron(&ron).ok()
    }

    /// Builds the parameter set for `doc` from the registry metadata. Unknown node
    /// kinds contribute nothing (the engine builder rejects them separately).
    #[must_use]
    pub fn for_doc(doc: &ProjectDoc, registry: &Registry) -> Self {
        let patch_ron = Mutex::new(vb_project::to_ron(doc).unwrap_or_default());
        // The id ↔ node-param ↔ host-name scheme lives in one place (vb-project's
        // mapping system); here we only turn each mapping into a nih-plug FloatParam.
        // Unknown kinds yield no parameters and so contribute nothing.
        let mappings = vb_project::build_mappings(doc, |kind| {
            registry.get(kind).map(|d| d.params()).unwrap_or_default()
        });
        let bound = mappings
            .into_iter()
            .map(|m| {
                let p = &m.param;
                // Rounded display so value → string → value round-trips exactly:
                // the digit budget shrinks as the range grows, keeping the printed
                // string well inside f32's ~7 significant digits.
                let decimals = if p.max.abs() >= 1_000.0 {
                    1
                } else if p.max.abs() >= 10.0 {
                    2
                } else {
                    3
                };
                let param = FloatParam::new(
                    m.name.clone(),
                    p.default,
                    FloatRange::Linear {
                        min: p.min,
                        max: p.max,
                    },
                )
                .with_value_to_string(formatters::v2s_f32_rounded(decimals));
                BoundParam {
                    node: m.node,
                    param,
                    id: m.id.0,
                }
            })
            .collect();
        Self { bound, patch_ron }
    }

    /// Current values grouped per node, in registry `params()` order — the shape
    /// [`engine_from_doc_with_values`] consumes.
    #[must_use]
    pub fn values_by_node(&self) -> HashMap<NodeId, Vec<f32>> {
        let mut map: HashMap<NodeId, Vec<f32>> = HashMap::new();
        for b in &self.bound {
            map.entry(b.node).or_default().push(b.param.value());
        }
        map
    }

    /// Number of automatable parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bound.len()
    }

    /// `(display name, current value)` for every automatable parameter, in host order —
    /// the read-only rows the plugin editor's run-view shows.
    #[must_use]
    pub fn overview(&self) -> Vec<(String, f32)> {
        self.bound
            .iter()
            .map(|b| (b.param.name().to_owned(), b.param.value()))
            .collect()
    }

    /// Whether the patch has no automatable parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bound.is_empty()
    }
}

// SAFETY: `param_map` returns one entry per `FloatParam` owned by `self.bound`; the
// pointers live exactly as long as `self`, ids are unique per construction, and the
// vector is never mutated after the plugin hands the Arc to the wrapper.
unsafe impl Params for VstBombParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        self.bound
            .iter()
            .map(|b| (b.id.clone(), b.param.as_ptr(), String::new()))
            .collect()
    }

    fn serialize_fields(&self) -> BTreeMap<String, String> {
        let mut fields = BTreeMap::new();
        if let Ok(ron) = self.patch_ron.lock() {
            if !ron.is_empty() {
                fields.insert("patch".to_owned(), ron.clone());
            }
        }
        fields
    }

    fn deserialize_fields(&self, serialized: &BTreeMap<String, String>) {
        if let (Some(ron), Ok(mut slot)) = (serialized.get("patch"), self.patch_ron.lock()) {
            // Only accept a patch that actually parses and validates; a corrupt
            // preset keeps the current graph instead of breaking the instance.
            if vb_project::open_ron(ron).is_ok() {
                *slot = ron.clone();
            }
        }
    }
}

/// Work the audio thread hands to nih-plug's background executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTask {
    /// Parameter values changed: rebuild the engine and swap it in.
    RebuildEngine,
}

/// Resource location of a patch shipped inside the bundle, given the dylib's path
/// (`…/Contents/MacOS/lib.dylib` → `…/Contents/Resources/patch.vbp`).
fn resources_patch_path(dylib: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos_dir = dylib.parent()?;
    let contents = macos_dir.parent()?;
    (macos_dir.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.join("Resources").join("patch.vbp"))
}

/// The path of the shared library this code lives in (`dladdr` on unix).
#[cfg(unix)]
fn own_dylib_path() -> Option<std::path::PathBuf> {
    let probe = own_dylib_path as *const std::os::raw::c_void;
    // SAFETY: dladdr only reads the address and fills the (zeroed) info struct;
    // dli_fname, when non-null, is a valid NUL-terminated path owned by the loader.
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(probe, &mut info) != 0 && !info.dli_fname.is_null() {
            let name = std::ffi::CStr::from_ptr(info.dli_fname);
            return Some(std::path::PathBuf::from(
                name.to_string_lossy().into_owned(),
            ));
        }
    }
    None
}

#[cfg(not(unix))]
fn own_dylib_path() -> Option<std::path::PathBuf> {
    None
}

/// The patch this instance should play, in priority order: `VB_PATCH` env override,
/// the patch bundled into the plugin's Resources (via `cargo xtask export`), then
/// the built-in default. Unreadable candidates fall through so the plugin loads.
fn load_patch(registry: &Registry) -> ProjectDoc {
    std::env::var("VB_PATCH")
        .ok()
        .and_then(|path| vb_project::open_path(std::path::Path::new(&path)).ok())
        .or_else(|| {
            own_dylib_path()
                .and_then(|dylib| resources_patch_path(&dylib))
                .and_then(|path| vb_project::open_path(&path).ok())
        })
        .unwrap_or_else(|| default_patch(registry))
}

impl Default for VstBomb {
    fn default() -> Self {
        let registry = Registry::with_builtins();
        let doc = load_patch(&registry);
        let params = VstBombParams::for_doc(&doc, &registry);
        let built_values = params.bound.iter().map(|b| b.param.value()).collect();
        let (tx, rx) = engine_swap();
        Self {
            params: Arc::new(params),
            doc: Arc::new(Mutex::new(Arc::new(doc))),
            registry: Arc::new(registry),
            build_ctx: BuildContext::default(),
            engine: None,
            sink: None,
            sample_rate: 48_000.0,
            frames: Arc::new(AtomicUsize::new(0)),
            rx,
            tx: Some(tx),
            built_values,
            #[cfg(feature = "gui")]
            editor_state: nih_plug_egui::EguiState::from_size(EDITOR_SIZE.0, EDITOR_SIZE.1),
        }
    }
}

/// The editor window's initial logical size (before DPI scaling). The node editor packs a
/// toolbar, two side panels (palette + inspector), and the canvas, so the old 560×380 was
/// cramped to the point of being unusable; this gives every panel real room. (Live
/// drag-resize is not wired: `nih_plug_egui`'s only resize entry point, `ResizableWindow`,
/// owns the whole `CentralPanel`, which conflicts with `EditorApp::render`'s multi-panel
/// layout — see the note in the PR. Enlarging the default is the safe, testable win.)
///
/// Gated on `gui` or `test` so the headless (no-editor) build neither carries nor warns on
/// it: under `gui` it feeds `from_size`, under `test` the roominess check reads it.
#[cfg(any(feature = "gui", test))]
const EDITOR_SIZE: (u32, u32) = (1120, 720);

impl Plugin for VstBomb {
    const NAME: &'static str = "VST_bomb";
    const VENDOR: &'static str = "ice1x";
    const URL: &'static str = "https://github.com/ice1x/VST_bomb";
    const EMAIL: &'static str = "tunodnb@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Instrument: no audio input, stereo output.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // Note on/off from the host reach the patch's midi-in / voice-in nodes.
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = PluginTask;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    /// The embedded node editor (000067 → full editor). Present only under the `gui`
    /// feature; the default build returns `None`, so a headless validator never opens a
    /// window. The executor lets an edit kick the off-audio-thread engine rebuild.
    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let _ = &async_executor;
        #[cfg(feature = "gui")]
        {
            editor::create(self.editor_state.clone(), self.doc.clone(), async_executor)
        }
        #[cfg(not(feature = "gui"))]
        {
            None
        }
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        // Runs off the audio thread: snapshot values, rebuild, swap. The audio
        // thread only ever polls the lock-free mailbox.
        let params = self.params.clone();
        let doc = self.doc.clone();
        let registry = self.registry.clone();
        let build_ctx = self.build_ctx.clone();
        let frames = self.frames.clone();
        let tx = self.tx.take().expect("task_executor is requested once");
        Box::new(move |task| match task {
            PluginTask::RebuildEngine => {
                let block = frames.load(Ordering::Acquire);
                if block == 0 {
                    return; // not initialized yet
                }
                let Ok(current_doc) = doc.lock().map(|d| d.clone()) else {
                    return;
                };
                tx.collect();
                let values = params.values_by_node();
                if let Ok((engine, _)) =
                    engine_from_doc_with_values(&current_doc, &registry, &build_ctx, block, &values)
                {
                    tx.install(engine);
                }
            }
        })
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        let frames = buffer_config.max_buffer_size as usize;
        self.frames.store(frames, Ordering::Release);
        // A state load may have delivered a different patch: nih-plug reinitializes
        // after set_state, so this is where the restored graph takes over.
        let current_doc = {
            let Ok(mut slot) = self.doc.lock() else {
                return false;
            };
            if let Some(restored) = self.params.stored_patch() {
                if vb_project::to_ron(&restored).ok() != vb_project::to_ron(&slot).ok() {
                    *slot = Arc::new(restored);
                }
            }
            slot.clone()
        };
        let values = self.params.values_by_node();
        for (slot, b) in self.built_values.iter_mut().zip(&self.params.bound) {
            *slot = b.param.value();
        }
        match engine_from_doc_with_values(
            &current_doc,
            &self.registry,
            &self.build_ctx,
            frames,
            &values,
        ) {
            Ok((engine, sink)) => {
                self.engine = Some(engine);
                self.sink = sink;
                true
            }
            Err(err) => {
                nih_error!("VST_bomb: patch failed to compile: {err}");
                false
            }
        }
    }

    fn reset(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(engine) = &mut self.engine else {
            return ProcessStatus::Error("engine not initialized");
        };
        // A background rebuild may have delivered a fresh engine.
        self.rx.poll_swap(engine);

        // Detect automation: compare current values against what the engine was
        // built with — in place, allocation-free — and kick a background rebuild.
        let mut changed = false;
        for (slot, b) in self.built_values.iter_mut().zip(&self.params.bound) {
            let value = b.param.value();
            if (*slot - value).abs() > f32::EPSILON {
                *slot = value;
                changed = true;
            }
        }
        if changed {
            context.execute_background(PluginTask::RebuildEngine);
        }

        // Drain the host's note events into the shared mailboxes; midi-in and
        // voice-in processors read them at the coming engine pass. Timing is
        // block-level for now (sample-accurate splitting is later polish).
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { note, velocity, .. } => {
                    route_note(&self.build_ctx, note, velocity, true);
                }
                NoteEvent::NoteOff { note, .. } => {
                    route_note(&self.build_ctx, note, 0.0, false);
                }
                _ => {}
            }
        }
        let frames = buffer.samples();
        engine.process(&vb_core::ProcessContext {
            sample_rate: self.sample_rate,
            frames,
        });

        let output = buffer.as_slice();
        match self.sink.and_then(|sink| engine.node_output(sink)) {
            Some(buf) => deliver_to_host(buf, output, frames),
            None => {
                for channel in output.iter_mut() {
                    channel[..frames].fill(0.0);
                }
            }
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for VstBomb {
    const CLAP_ID: &'static str = "com.ice1x.vst-bomb";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Node-graph synthesizer built with VST_bomb");
    const CLAP_MANUAL_URL: Option<&'static str> = Some("https://github.com/ice1x/VST_bomb");
    const CLAP_SUPPORT_URL: Option<&'static str> = Some("https://github.com/ice1x/VST_bomb/issues");
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

/// Routes one host note event into both note mailboxes (mono + poly).
///
/// nih-plug velocities are `[0, 1]`; the mailboxes speak MIDI bytes, so the value is
/// scaled to `0..=127` with rounding and clamping.
fn route_note(ctx: &BuildContext, note: u8, velocity: f32, on: bool) {
    if on {
        let vel = (velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
        ctx.notes.note_on(note, vel);
        ctx.poly.note_on(note, vel);
    } else {
        ctx.notes.note_off(note);
        ctx.poly.note_off(note);
    }
}

/// Copies the sink node's output onto the host's output channels.
///
/// A mono sink (one channel) is duplicated across every host channel — the historical
/// behavior, and what every patch ending in a single-output node still produces. A
/// multi-channel sink (e.g. a `pan` node's `left`/`right`) maps channel-for-channel, so a
/// patch that ends in a stereo node feeds the host true stereo. If the host has more
/// channels than the sink, the sink's last channel is reused; a zero-channel sink is
/// silence.
fn deliver_to_host(sink: &vb_core::AudioBuffer, host: &mut [&mut [f32]], frames: usize) {
    let nch = sink.channels();
    for (h, channel) in host.iter_mut().enumerate() {
        let src = if nch == 0 {
            None
        } else if nch == 1 {
            sink.channel(0)
        } else {
            sink.channel(h.min(nch - 1))
        };
        match src {
            Some(src) => {
                let n = frames.min(src.len()).min(channel.len());
                channel[..n].copy_from_slice(&src[..n]);
            }
            None => {
                let n = frames.min(channel.len());
                channel[..n].fill(0.0);
            }
        }
    }
}

nih_export_clap!(VstBomb);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_window_is_roomy_enough_for_the_node_editor() {
        // The embedded editor packs a toolbar, a palette and an inspector side panel, and the
        // canvas. The old 560×380 default crammed all of that into a few inches; guard against
        // ever regressing to a window too small to actually use.
        let (w, h) = EDITOR_SIZE;
        assert!(
            w >= 1000,
            "editor window is only {w} px wide — too narrow for two side panels + a canvas"
        );
        assert!(h >= 640, "editor window is only {h} px tall — too short");
    }

    #[test]
    fn deliver_to_host_duplicates_a_mono_sink() {
        // One-channel sink → every host channel gets the same signal (legacy behavior).
        let mut sink = vb_core::AudioBuffer::new(1, 4);
        sink.channel_mut(0)
            .unwrap()
            .copy_from_slice(&[0.1, 0.2, 0.3, 0.4]);
        let mut l = [0.0_f32; 4];
        let mut r = [0.0_f32; 4];
        {
            let mut host: [&mut [f32]; 2] = [&mut l, &mut r];
            deliver_to_host(&sink, &mut host, 4);
        }
        assert_eq!(l, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(
            r,
            [0.1, 0.2, 0.3, 0.4],
            "mono is duplicated to both channels"
        );
    }

    #[test]
    fn deliver_to_host_maps_a_stereo_sink_channel_for_channel() {
        // Two-channel sink (e.g. a pan node) → host L←ch0, R←ch1: true stereo.
        let mut sink = vb_core::AudioBuffer::new(2, 3);
        sink.channel_mut(0)
            .unwrap()
            .copy_from_slice(&[1.0, 1.0, 1.0]);
        sink.channel_mut(1)
            .unwrap()
            .copy_from_slice(&[-1.0, -1.0, -1.0]);
        let mut l = [0.0_f32; 3];
        let mut r = [0.0_f32; 3];
        {
            let mut host: [&mut [f32]; 2] = [&mut l, &mut r];
            deliver_to_host(&sink, &mut host, 3);
        }
        assert_eq!(l, [1.0, 1.0, 1.0], "left ← sink channel 0");
        assert_eq!(r, [-1.0, -1.0, -1.0], "right ← sink channel 1");
    }

    #[test]
    fn deliver_to_host_reuses_the_last_channel_for_extra_host_channels() {
        // A stereo sink into a 3-channel host: the third host channel reuses sink ch1.
        let mut sink = vb_core::AudioBuffer::new(2, 2);
        sink.channel_mut(0).unwrap().copy_from_slice(&[0.5, 0.5]);
        sink.channel_mut(1).unwrap().copy_from_slice(&[0.9, 0.9]);
        let mut a = [0.0_f32; 2];
        let mut b = [0.0_f32; 2];
        let mut c = [0.0_f32; 2];
        {
            let mut host: [&mut [f32]; 3] = [&mut a, &mut b, &mut c];
            deliver_to_host(&sink, &mut host, 2);
        }
        assert_eq!(a, [0.5, 0.5]);
        assert_eq!(b, [0.9, 0.9]);
        assert_eq!(
            c,
            [0.9, 0.9],
            "extra host channel reuses the last sink channel"
        );
    }

    #[test]
    fn the_plugin_constructs_with_the_default_patch() {
        let plugin = VstBomb::default();
        let doc = plugin.doc.lock().unwrap().clone();
        assert!(!doc.nodes.is_empty(), "default patch has nodes to play");
        assert!(plugin.engine.is_none(), "engine waits for initialize");
    }

    #[test]
    fn host_notes_reach_both_mailboxes() {
        let ctx = BuildContext::default();

        route_note(&ctx, 69, 1.0, true);
        assert_eq!(
            ctx.notes.read(),
            (true, 69, 127),
            "full velocity maps to 127"
        );
        assert_eq!(ctx.poly.read(0), (true, 69, 127));

        route_note(&ctx, 72, 0.5, true);
        let (_, _, vel) = ctx.poly.read(1);
        assert_eq!(vel, 64, "0.5 rounds to 64");

        route_note(&ctx, 69, 0.0, false);
        assert!(
            !ctx.notes.read().0 || ctx.notes.read().1 != 69,
            "mono released or replaced"
        );
        assert!(!ctx.poly.read(0).0, "voice 0 released");
        assert!(ctx.poly.read(1).0, "voice 1 still sounding");

        // Out-of-range velocities clamp instead of wrapping.
        route_note(&ctx, 60, 7.5, true);
        let (_, _, vel) = ctx.poly.read(0);
        assert_eq!(vel, 127);
    }

    #[test]
    fn params_mirror_the_patch_and_have_stable_unique_ids() {
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let params = VstBombParams::for_doc(&doc, &registry);

        // osc (waveform, freq_hz) + svf (cutoff_hz, q) + adsr (a, d, s, r) +
        // vca (bias, amount) = 10 knobs; midi-in contributes none.
        assert_eq!(params.len(), 10);
        let map = params.param_map();
        let ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "ids are unique: {ids:?}");
        assert!(ids.iter().all(|id| id.starts_with('n')), "stable id scheme");
    }

    #[test]
    fn value_snapshot_groups_by_node_in_registry_order() {
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let params = VstBombParams::for_doc(&doc, &registry);
        let values = params.values_by_node();

        assert_eq!(
            values.len(),
            4,
            "osc, svf, adsr and vca carry parameters (midi-in has none)"
        );
        // Each group matches the mapping's defaults: the registry defaults, except
        // where the patch stores its own values (the VCA, biased shut at bias = 0).
        for entry in &doc.nodes {
            let params = registry.get(&entry.node.kind).unwrap().params();
            if params.is_empty() {
                continue;
            }
            let stored = doc.node_values(entry.id);
            let expected: Vec<f32> = params
                .iter()
                .enumerate()
                .map(|(i, p)| stored.get(i).copied().unwrap_or(p.default))
                .collect();
            assert_eq!(values[&entry.id], expected, "{}", entry.node.kind);
        }
    }

    #[test]
    fn engine_rebuild_with_values_changes_the_sound() {
        let registry = Registry::with_builtins();
        // A plain oscillator → gain voice: gain is the terminal and scales the level
        // linearly, so an automated gain value shows up directly in the peak.
        let mut graph = vb_core::Graph::new();
        let osc = graph.add_node(registry.graph_node("oscillator").unwrap());
        let amp = graph.add_node(registry.graph_node("gain").unwrap());
        graph
            .connect(osc, vb_core::PortId(0), amp, vb_core::PortId(0))
            .unwrap();
        let doc = ProjectDoc::from_graph(&graph);
        let ctx = BuildContext::default();
        let gain_node = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "gain")
            .map(|e| e.id)
            .unwrap();

        let run = |values: &HashMap<NodeId, Vec<f32>>| {
            let (mut engine, sink) =
                engine_from_doc_with_values(&doc, &registry, &ctx, 256, values).unwrap();
            let sink = sink.unwrap();
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 256,
            });
            engine
                .node_output(sink)
                .unwrap()
                .channel(0)
                .unwrap()
                .iter()
                .fold(0.0_f32, |m, s| m.max(s.abs()))
        };

        let loud = run(&HashMap::from([(gain_node, vec![1.0])]));
        let quiet = run(&HashMap::from([(gain_node, vec![0.25])]));
        assert!(
            (quiet - loud * 0.25).abs() < 1e-3,
            "automated gain reaches the processor: {loud} vs {quiet}"
        );
    }

    #[test]
    fn preset_state_carries_the_whole_patch() {
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let params = VstBombParams::for_doc(&doc, &registry);

        // Save: the serialized fields embed the patch RON.
        let fields = params.serialize_fields();
        let ron = fields.get("patch").expect("patch field present");
        assert!(vb_project::open_ron(ron).is_ok(), "stored patch is valid");

        // Restore into a fresh instance built around a DIFFERENT (smaller) patch.
        let mut other_graph = vb_core::Graph::new();
        other_graph.add_node(registry.graph_node("oscillator").unwrap());
        let other_doc = ProjectDoc::from_graph(&other_graph);
        let fresh = VstBombParams::for_doc(&other_doc, &registry);
        fresh.deserialize_fields(&fields);

        let restored = fresh.stored_patch().expect("patch restored");
        assert_eq!(
            restored.nodes.len(),
            doc.nodes.len(),
            "full graph came back"
        );
        assert_eq!(restored.edges.len(), doc.edges.len());
    }

    #[test]
    fn corrupt_preset_patches_are_rejected() {
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let params = VstBombParams::for_doc(&doc, &registry);
        let before = params.stored_patch().expect("valid initial patch");

        let mut corrupt = BTreeMap::new();
        corrupt.insert("patch".to_owned(), "garbage(".to_owned());
        params.deserialize_fields(&corrupt);

        let after = params.stored_patch().expect("still the previous patch");
        assert_eq!(
            before.nodes.len(),
            after.nodes.len(),
            "corrupt state ignored"
        );
    }

    #[test]
    fn bundle_resource_path_is_derived_from_the_dylib_location() {
        use std::path::Path;
        let dylib = Path::new("/Library/Audio/vb.clap/Contents/MacOS/vb-plugin");
        assert_eq!(
            resources_patch_path(dylib),
            Some("/Library/Audio/vb.clap/Contents/Resources/patch.vbp".into())
        );
        // A bare dylib outside a bundle has no resource patch.
        assert_eq!(
            resources_patch_path(Path::new("/usr/lib/libfoo.dylib")),
            None
        );
    }

    #[test]
    fn engine_compiles_at_host_buffer_sizes() {
        // The initialize path without the nih-plug harness: same builder, same doc.
        let plugin = VstBomb::default();
        let doc = plugin.doc.lock().unwrap().clone();
        for frames in [64_usize, 256, 1_024, 4_096] {
            let built = engine_from_doc(&doc, &plugin.registry, &plugin.build_ctx, frames);
            assert!(built.is_ok(), "buffer size {frames}");
        }
    }
}
