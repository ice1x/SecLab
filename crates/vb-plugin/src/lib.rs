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

    /// Replaces the serialized patch that the host saves in its project state.
    ///
    /// The editor calls this whenever the in-window patch changes — including **view-only**
    /// edits (panel placement, control type) that never rebuild the engine — so a DAW project
    /// saved after arranging the panel reopens with that arrangement. Without it, the host state
    /// would keep the patch as it was at load and every in-plugin edit would be lost on reload.
    pub fn set_patch_ron(&self, ron: String) {
        if let Ok(mut slot) = self.patch_ron.lock() {
            *slot = ron;
        }
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

    /// Flat index into `bound` for the parameter at registry position `index` of `node`,
    /// or `None` when that node/index is not host-automatable (e.g. a node added in-session,
    /// whose params are not exposed until reload). Routes an editor knob edit to the matching
    /// host parameter.
    fn bound_index_of(&self, node: NodeId, index: usize) -> Option<usize> {
        let mut seen = 0usize;
        for (bi, b) in self.bound.iter().enumerate() {
            if b.node == node {
                if seen == index {
                    return Some(bi);
                }
                seen += 1;
            }
        }
        None
    }

    /// Moves the host parameter bound to `(node, index)` to `value` through `setter`,
    /// wrapping it in a one-shot host gesture so the DAW records the editor knob move.
    ///
    /// This is the fix for editor knobs not reaching the audio engine: the engine rebuild
    /// sources every node's values from [`values_by_node`](Self::values_by_node) (the host
    /// parameter snapshot), and that override wins over the document's stored values (see
    /// [`engine_from_doc_with_values`]). So an in-editor edit that only touched the document
    /// was silently reverted on the next rebuild. Writing the edit through to the host
    /// parameter makes the snapshot — and therefore the rebuilt engine — reflect it.
    ///
    /// A no-op (returns `false`) for a `(node, index)` that is not automatable.
    pub fn set_bound(&self, setter: &ParamSetter, node: NodeId, index: usize, value: f32) -> bool {
        let Some(bi) = self.bound_index_of(node, index) else {
            return false;
        };
        let param = &self.bound[bi].param;
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
        true
    }
}

/// Parameters whose value changed between two per-node value snapshots (each a node id →
/// values in registry `params()` order, the shape [`VstBombParams::values_by_node`] and
/// [`ProjectDoc`] both use): returns `(node, index, new_value)` for each.
///
/// Drives the plugin's editor→host parameter write-through so **only** the knob the user
/// actually moved in the editor is pushed to the host — parameters the editor never touched
/// (for example one moved by host automation) do not appear here and are left alone. A value
/// present in `next` but absent from `prev` counts as changed.
#[must_use]
pub fn changed_values(
    prev: &HashMap<NodeId, Vec<f32>>,
    next: &HashMap<NodeId, Vec<f32>>,
) -> Vec<(NodeId, usize, f32)> {
    let mut out = Vec::new();
    for (node, values) in next {
        let before = prev.get(node);
        for (index, &value) in values.iter().enumerate() {
            let changed = match before.and_then(|b| b.get(index)) {
                Some(&old) => (old - value).abs() > f32::EPSILON,
                None => true,
            };
            if changed {
                out.push((*node, index, value));
            }
        }
    }
    out
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
            editor::create(
                self.editor_state.clone(),
                self.params.clone(),
                self.doc.clone(),
                async_executor,
            )
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

        // Detect a parameter change (automation, or a knob moved in the editor) and apply it to
        // the RUNNING engine in place — no rebuild, no engine swap — so the sound retunes
        // smoothly instead of clicking. Turning a knob during playback used to rebuild the whole
        // engine every block, resetting every processor's state (oscillator phase, filter memory,
        // envelope stage) and corrupting the sound during the turn. Now each changed node is
        // live-updated via `RtEngine::set_doc_params`, addressed by its document id.
        //
        // A node whose processor does not support a live update returns `false`; only then do we
        // fall back to a full background rebuild (also the path for structural edits). Parameters
        // are grouped by node contiguously in `bound` (built per node, in registry order), so this
        // gathers each node's full value slice on the stack — allocation-free, audio-thread safe.
        const MAX_LIVE_PARAMS: usize = 32;
        let bound = &self.params.bound;
        let mut needs_rebuild = false;
        let mut i = 0;
        while i < bound.len() {
            let node = bound[i].node;
            let mut vals = [0.0_f32; MAX_LIVE_PARAMS];
            let mut n = 0;
            let mut node_changed = false;
            let mut j = i;
            while j < bound.len() && bound[j].node == node {
                let value = bound[j].param.value();
                if (self.built_values[j] - value).abs() > f32::EPSILON {
                    node_changed = true;
                }
                if n < MAX_LIVE_PARAMS {
                    vals[n] = value;
                    n += 1;
                }
                j += 1;
            }
            if node_changed {
                // A node with more than MAX_LIVE_PARAMS parameters could not be fully applied on
                // the stack; rebuild it instead of live-updating a truncated slice.
                let live = n <= MAX_LIVE_PARAMS && engine.set_doc_params(node, &vals[..n]);
                if !live {
                    needs_rebuild = true;
                }
                // Sync built_values for this node's run to the current values.
                for (k, b) in bound[i..j].iter().enumerate() {
                    self.built_values[i + k] = b.param.value();
                }
            }
            i = j;
        }
        if needs_rebuild {
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
        // Process the host block in fixed control-rate quanta so block-rate modulation (the
        // ladder cutoff, note gates) resolves the same way regardless of the host buffer size —
        // realtime and offline bounce then sound identical.
        let frames = buffer.samples();
        let sink = self.sink;
        let sample_rate = self.sample_rate;
        let output = buffer.as_slice();
        process_in_quanta(engine, sink, sample_rate, frames, output);
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
/// Whole-block convenience over [`deliver_to_host_at`] (offset 0), used by the tests.
#[cfg(test)]
fn deliver_to_host(sink: &vb_core::AudioBuffer, host: &mut [&mut [f32]], frames: usize) {
    deliver_to_host_at(sink, host, 0, frames);
}

/// Like [`deliver_to_host`], but writes the sink's `frames` samples into each host channel
/// starting at `offset` — the entry point the sub-block loop uses to place each quantum's
/// output at its place in the full host buffer.
fn deliver_to_host_at(
    sink: &vb_core::AudioBuffer,
    host: &mut [&mut [f32]],
    offset: usize,
    frames: usize,
) {
    let nch = sink.channels();
    for (h, channel) in host.iter_mut().enumerate() {
        let src = if nch == 0 {
            None
        } else if nch == 1 {
            sink.channel(0)
        } else {
            sink.channel(h.min(nch - 1))
        };
        let room = channel.len().saturating_sub(offset);
        match src {
            Some(src) => {
                let n = frames.min(src.len()).min(room);
                channel[offset..offset + n].copy_from_slice(&src[..n]);
            }
            None => {
                let n = frames.min(room);
                channel[offset..offset + n].fill(0.0);
            }
        }
    }
}

/// Number of samples the engine advances per pass. Control-rate modulation (an envelope on the
/// [`vb_nodes::Ladder`] cutoff, note gates, and any node that reads its control inputs once per
/// pass) updates at this granularity, so the host's buffer size is split into quanta of this
/// length before processing. That keeps the sound **independent of the host buffer**: a Logic
/// bounce at a large offline buffer resolves the filter's attack sweep exactly as realtime
/// playback at a small buffer does, instead of stepping the cutoff coarsely and thickening the
/// low-frequency attack. 64 samples ≈ 1.3 ms at 48 kHz — fine enough to be inaudible, coarse
/// enough to stay cheap.
const CONTROL_QUANTUM: usize = 64;

/// Runs `engine` over `frames` samples in [`CONTROL_QUANTUM`]-sized passes, writing the `sink`
/// node's output into `host` at the matching offset. Splitting the host block into fixed quanta
/// is what makes block-rate modulation independent of the host buffer size.
fn process_in_quanta(
    engine: &mut RtEngine,
    sink: Option<NodeId>,
    sample_rate: f32,
    frames: usize,
    host: &mut [&mut [f32]],
) {
    let mut done = 0;
    while done < frames {
        let n = CONTROL_QUANTUM.min(frames - done);
        engine.process(&vb_core::ProcessContext {
            sample_rate,
            frames: n,
        });
        match sink.and_then(|s| engine.node_output(s)) {
            Some(buf) => deliver_to_host_at(buf, host, done, n),
            None => {
                for channel in host.iter_mut() {
                    let end = (done + n).min(channel.len());
                    channel[done..end].fill(0.0);
                }
            }
        }
        done += n;
    }
}

nih_export_clap!(VstBomb);

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the first `frames` samples of the default (Minitaur) patch after a low note-on,
    /// either as one engine pass (`chunked = false`, the old behaviour = one host block) or in
    /// [`CONTROL_QUANTUM`] quanta (`chunked = true`, the new behaviour). A fresh engine each
    /// call so the attack starts from reset.
    fn render_default_attack(frames: usize, chunked: bool) -> Vec<f32> {
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let build_ctx = BuildContext::default();
        let (mut engine, sink) =
            engine_from_doc(&doc, &registry, &build_ctx, frames.max(CONTROL_QUANTUM)).unwrap();
        route_note(&build_ctx, 45, 1.0, true); // a low bass note, where the filter sweep shows
        let mut ch = vec![0.0_f32; frames];
        {
            let mut host: [&mut [f32]; 1] = [&mut ch];
            if chunked {
                process_in_quanta(&mut engine, sink, 48_000.0, frames, &mut host);
            } else {
                engine.process(&vb_core::ProcessContext {
                    sample_rate: 48_000.0,
                    frames,
                });
                if let Some(buf) = sink.and_then(|s| engine.node_output(s)) {
                    deliver_to_host(buf, &mut host, frames);
                }
            }
        }
        ch
    }

    #[test]
    fn quanta_match_a_single_pass_when_the_block_fits_one_quantum() {
        // A host block no larger than one quantum must behave exactly like a single pass —
        // splitting changes nothing, so small-buffer realtime is untouched.
        let single = render_default_attack(CONTROL_QUANTUM, false);
        let quantised = render_default_attack(CONTROL_QUANTUM, true);
        assert_eq!(single, quantised);
    }

    #[test]
    fn quanta_resolve_the_filter_attack_independently_of_block_size() {
        // The bug the user heard: one big offline block retunes the ladder cutoff once for the
        // whole buffer, thickening the low-frequency attack, while realtime's small blocks
        // resolve the sweep finely. Processing a big block in quanta must reproduce the fine
        // resolution — so the quantised render differs from the single-block render in the
        // attack, and both are audible.
        let one_block = render_default_attack(1024, false);
        let quantised = render_default_attack(1024, true);
        assert!(one_block.iter().all(|s| s.is_finite()));
        assert!(quantised.iter().all(|s| s.is_finite()));
        assert!(
            quantised[..512].iter().map(|s| s.abs()).sum::<f32>() > 0.0,
            "the note must sound"
        );
        let attack_diff: f32 = one_block[..512]
            .iter()
            .zip(&quantised[..512])
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            attack_diff > 0.01,
            "quantising must change the block-rate filter attack: {attack_diff}"
        );
    }

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

    /// A small, fixed voice (oscillator → svf → vca, note-gated) with a KNOWN parameter
    /// shape — 10 knobs across 4 param-bearing nodes — so the parameter-mapping tests assert
    /// the mechanism independently of whatever `default_patch` currently is (it became the
    /// larger Minitaur voice in 000164).
    fn sample_voice_doc(registry: &Registry) -> vb_project::ProjectDoc {
        use vb_core::PortId;
        let mut g = vb_core::Graph::new();
        let osc = g.add_node(registry.graph_node("oscillator").expect("builtin"));
        let svf = g.add_node(registry.graph_node("svf").expect("builtin"));
        let midi = g.add_node(registry.graph_node("midi-in").expect("builtin"));
        let env = g.add_node(registry.graph_node("adsr").expect("builtin"));
        let vca = g.add_node(registry.graph_node("vca").expect("builtin"));
        g.connect(osc, PortId(0), svf, PortId(0)).unwrap();
        g.connect(svf, PortId(0), vca, PortId(0)).unwrap();
        g.connect(midi, PortId(0), env, PortId(0)).unwrap();
        g.connect(env, PortId(0), vca, PortId(1)).unwrap();
        let mut doc = vb_project::ProjectDoc::from_graph(&g);
        doc.set_node_values(vca, vec![0.0, 1.0]);
        doc
    }

    #[test]
    fn params_mirror_the_patch_and_have_stable_unique_ids() {
        let registry = Registry::with_builtins();
        let doc = sample_voice_doc(&registry);
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
        let doc = sample_voice_doc(&registry);
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
    fn changed_values_reports_only_the_moved_knob() {
        let node = NodeId(7);
        let prev = HashMap::from([(node, vec![1.0, 2.0, 3.0])]);
        let next = HashMap::from([(node, vec![1.0, 9.0, 3.0])]);
        assert_eq!(changed_values(&prev, &next), vec![(node, 1, 9.0)]);
    }

    #[test]
    fn changed_values_is_empty_when_nothing_moved() {
        let snap = HashMap::from([(NodeId(1), vec![0.5, 0.6]), (NodeId(2), vec![1.0])]);
        assert!(changed_values(&snap, &snap).is_empty());
    }

    #[test]
    fn changed_values_treats_a_new_value_as_changed() {
        let node = NodeId(3);
        let prev = HashMap::new();
        let next = HashMap::from([(node, vec![0.25])]);
        assert_eq!(changed_values(&prev, &next), vec![(node, 0, 0.25)]);
    }

    #[test]
    fn bound_index_routes_each_node_param_to_its_own_binding() {
        let registry = Registry::with_builtins();
        let doc = sample_voice_doc(&registry);
        let params = VstBombParams::for_doc(&doc, &registry);

        // The ADSR carries four params (a, d, s, r): each index must resolve to a distinct
        // binding that belongs to that node, and an out-of-range index resolves to nothing.
        let adsr = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "adsr")
            .map(|e| e.id)
            .expect("sample voice has an adsr");
        let mut seen = Vec::new();
        for index in 0..4 {
            let bi = params
                .bound_index_of(adsr, index)
                .expect("adsr param is automatable");
            assert_eq!(params.bound[bi].node, adsr, "binding belongs to the node");
            seen.push(bi);
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 4, "four distinct bindings for four params");
        assert_eq!(
            params.bound_index_of(adsr, 4),
            None,
            "index past the last param"
        );
        assert_eq!(
            params.bound_index_of(NodeId(9_999), 0),
            None,
            "unknown node"
        );
    }

    #[test]
    fn an_editor_knob_edit_reaches_the_rebuilt_engine() {
        // The exact bug the user hit: dragging a knob in the editor changed the document but
        // the rebuild override (the host-param snapshot) reverted it. The fix pushes the edit
        // into the host param so the snapshot — and the rebuilt engine — carry it. Here we
        // stand in for the ParamSetter by applying `changed_values` straight into the snapshot,
        // proving the reconciliation the write-through relies on actually moves the sound.
        let registry = Registry::with_builtins();
        let mut graph = vb_core::Graph::new();
        let osc = graph.add_node(registry.graph_node("oscillator").unwrap());
        let amp = graph.add_node(registry.graph_node("gain").unwrap());
        graph
            .connect(osc, vb_core::PortId(0), amp, vb_core::PortId(0))
            .unwrap();
        let doc = ProjectDoc::from_graph(&graph);
        let ctx = BuildContext::default();
        let params = VstBombParams::for_doc(&doc, &registry);
        let gain = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "gain")
            .map(|e| e.id)
            .unwrap();

        // Snapshot before the edit is the host-param default (gain = 1.0).
        let before = params.values_by_node();
        // The editor drags gain down to 0.25: `to_doc` would produce this snapshot.
        let mut edited = before.clone();
        *edited.get_mut(&gain).unwrap().get_mut(0).unwrap() = 0.25;

        // Only the gain knob is reported as moved.
        let moved = changed_values(&before, &edited);
        assert_eq!(moved, vec![(gain, 0, 0.25)]);

        // Apply the move into the rebuild override (what set_bound achieves via the host
        // param) and rebuild: the engine now runs at the edited gain, not the stale default.
        let mut snapshot = before.clone();
        for (node, index, value) in moved {
            snapshot.get_mut(&node).unwrap()[index] = value;
        }
        let (mut engine, sink) =
            engine_from_doc_with_values(&doc, &registry, &ctx, 256, &snapshot).unwrap();
        let sink = sink.unwrap();
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let peak = engine
            .node_output(sink)
            .unwrap()
            .channel(0)
            .unwrap()
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            peak <= 0.26,
            "edited gain applied, not reverted: peak {peak}"
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
    fn a_default_patch_node_updates_live_by_document_id() {
        // The engine built from a document aliases document ids, so the plugin can retune a
        // running node in place (no rebuild, no click) via its document id — the fix for turning
        // a knob during playback corrupting the sound.
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let ctx = BuildContext::default();
        let (mut engine, _sink) =
            engine_from_doc_with_values(&doc, &registry, &ctx, 128, &HashMap::new()).unwrap();

        let ladder = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "ladder")
            .map(|e| e.id)
            .expect("default patch has a ladder");
        assert!(
            engine.set_doc_params(ladder, &[500.0, 0.5, 2_000.0]),
            "the ladder retunes live by its document id"
        );

        // midi-in carries no params (default no-op set_params → false), and an unknown id is false.
        let midi = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "midi-in")
            .map(|e| e.id)
            .expect("default patch has a midi-in");
        assert!(
            !engine.set_doc_params(midi, &[0.0]),
            "midi-in has no live params"
        );
        assert!(!engine.set_doc_params(NodeId(99_999), &[0.0]), "unknown id");
    }

    #[test]
    fn set_patch_ron_updates_the_saved_host_state() {
        // The editor persists an in-window edit (here: a smaller patch) into the params; the
        // host's saved state must then carry that edited patch, not the one loaded at construction
        // — this is what lets a rearranged panel survive a DAW project save/reopen.
        let registry = Registry::with_builtins();
        let params = VstBombParams::for_doc(&default_patch(&registry), &registry);

        let mut edited = vb_core::Graph::new();
        edited.add_node(registry.graph_node("oscillator").unwrap());
        let edited_doc = ProjectDoc::from_graph(&edited);
        params.set_patch_ron(vb_project::to_ron(&edited_doc).unwrap());

        let fields = params.serialize_fields();
        let saved = fields.get("patch").expect("patch field present");
        let restored = vb_project::open_ron(saved).expect("saved patch parses");
        assert_eq!(restored.nodes.len(), 1, "the host saves the edited patch");
        assert_eq!(params.stored_patch().unwrap().nodes.len(), 1);
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
