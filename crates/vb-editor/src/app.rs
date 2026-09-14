//! The editor window shell: an infinite canvas with pan, zoom, a grid, and nodes.
//!
//! Input mapping (Reaktor-style):
//! - drag on empty canvas — pan; scroll / pinch — zoom anchored at the pointer
//! - drag from a port — pull a wire; drop on a compatible port to connect
//! - drag a node's title/body — move it; click — select (raises), click empty to clear
//! - Delete/Backspace — remove the selection; Cmd/Ctrl+D — duplicate it
//! - Cmd/Ctrl+Z — undo; Shift+Cmd/Ctrl+Z — redo (one step per completed gesture)
//!
//! This shell owns the [`Camera`] and the [`PatchView`] everything draws through;
//! `vb-standalone` embeds it and keeps a live audio engine in sync with the patch.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use vb_core::{Parameter, PortDirection, PortId};
// Only the test-only `sample_wiring_patch` and the wiring tests hand-build nodes now that the
// demo patch is loaded from RON.
#[cfg(test)]
use vb_core::{GraphNode, Port, SignalType};

use vb_nodes::Registry;

use std::path::{Path, PathBuf};

use crate::camera::{grid_step, Camera};
use crate::layout::{layout_node, NodeLayout, NodeStyle};
use crate::library::{library_items_from_module_dir, LibraryBrowser};
use crate::library_panel::library_browser_ui;
use crate::module::{list_modules, MacroModule, ModuleError};
use crate::palette::palette_entries;
use crate::patch::PatchView;
use crate::theme::{Theme, ThemeKind};
use crate::widgets::{
    self, switch_index, switch_step, switch_values, xy_norm_to_pad, xy_pad_to_norm, WidgetKind,
};

#[cfg(feature = "assistant")]
use crate::assistant_panel::{assistant_panel_ui, AssistantAction};
#[cfg(feature = "assistant")]
use crate::settings_panel::{settings_panel_ui, SettingsAction};
#[cfg(feature = "assistant")]
use std::sync::{mpsc, Arc};
#[cfg(feature = "assistant")]
use std::thread::JoinHandle;
#[cfg(feature = "assistant")]
use vb_ai::{
    AiProvider, AssistantSettings, EditOutcome, EditSession, EditState, GenerateError, KeyStore,
};

/// A background provider slot: any `AiProvider` that can cross a thread boundary. The
/// concrete network providers ([`vb_ai::AnthropicProvider`], [`vb_ai::LocalProvider`]) are
/// `Send + Sync`; a host injects one via [`EditorApp::set_assistant_provider`].
#[cfg(feature = "assistant")]
type SharedProvider = Arc<dyn AiProvider + Send + Sync>;

/// The message a worker thread sends back: the request it ran and the outcome of
/// `edit_patch` against the snapshot it was given.
#[cfg(feature = "assistant")]
type ProposalMsg = (String, Result<EditOutcome, GenerateError>);

/// The native AI assistant, wired into [`EditorApp`] behind the `assistant` feature
/// (backlog 000108). It owns a live [`EditSession`] working copy seeded from the editor's
/// patch, the input draft, and a **provider slot**.
///
/// `EditSession::submit` is synchronous and [`vb_ai::ReqwestTransport`] is blocking, so the
/// provider call runs on a **worker thread**: Submit snapshots the working-copy document and
/// spawns `edit_patch` on it; each frame polls the worker's channel and, when the proposal
/// arrives, records it on the main thread via [`EditSession::record`] — the session never
/// crosses a thread. The slot is empty until a provider is injected; with the `assistant-net`
/// feature, [`EditorApp::set_assistant_anthropic`] / [`EditorApp::set_assistant_local`]
/// construct real ones from the OS keychain.
#[cfg(feature = "assistant")]
struct AssistantHost {
    /// Working copy the assistant edits. Seeded from the editor's patch the first time the
    /// panel is shown, and re-seeded from the live patch whenever a fresh request starts
    /// with no proposal pending, so direct canvas edits are picked up. `None` until shown.
    session: Option<EditSession>,
    /// The LLM backend, or `None` when none is configured (the panel then reports so).
    provider: Option<SharedProvider>,
    /// Node kinds the worker validates proposals against — the builtins registry, shared into
    /// each worker thread (an `Arc` because [`vb_nodes::Registry`] is not `Clone`).
    registry: Arc<Registry>,
    /// The Settings view-model the panel edits — provider/model/sampling + privacy. Its
    /// [`AssistantSettings::config`] is the single source the worker snapshots per request.
    settings: AssistantSettings,
    /// BYOK key storage: the OS keychain (service `"VST_bomb"`) under `assistant-net`, else an
    /// in-session in-memory store. Read/written only through the Settings panel.
    keys: Box<dyn KeyStore>,
    /// Scratch buffer for the masked key field in the Settings panel.
    key_draft: String,
    /// The in-progress instruction text.
    draft: String,
    /// Repair attempts the session may spend converging a valid patch per request.
    max_attempts: u32,
    /// Last submit/apply/reject outcome, shown under the panel.
    status: Option<String>,
    /// The channel a request-in-flight worker reports back on; `Some` iff a request is
    /// currently running on a worker thread.
    rx: Option<mpsc::Receiver<ProposalMsg>>,
    /// The in-flight worker's handle, joined when its message is consumed.
    worker: Option<JoinHandle<()>>,
}

#[cfg(feature = "assistant")]
impl AssistantHost {
    /// A fresh host with no backend, the builtins registry, and a sensible repair budget.
    fn new() -> Self {
        // Persist keys in the OS keychain when the network providers are compiled in;
        // otherwise a key lives only for the session (still enough to drive an injected
        // provider in the base build/tests).
        #[cfg(feature = "assistant-net")]
        let keys: Box<dyn KeyStore> = Box::new(vb_ai::keystore::KeychainStore::new("VST_bomb"));
        #[cfg(not(feature = "assistant-net"))]
        let keys: Box<dyn KeyStore> = Box::new(vb_ai::MemoryKeyStore::new());
        Self {
            session: None,
            provider: None,
            registry: Arc::new(Registry::with_builtins()),
            settings: AssistantSettings::new(),
            keys,
            key_draft: String::new(),
            draft: String::new(),
            max_attempts: 3,
            status: None,
            rx: None,
            worker: None,
        }
    }

    /// Whether a request is currently running on a worker thread.
    fn in_flight(&self) -> bool {
        self.rx.is_some()
    }
}

/// The physical unit a parameter is measured in, inferred from its name suffix (the registry
/// encodes units by convention: `cutoff_hz`, `attack_s`, `glide_ms`, `threshold_db`,
/// `detune_semitones`). Returns `""` for unitless parameters (mix, resonance, bias, …). The
/// inspector shows it next to the parameter name so a knob reads in real terms.
fn param_unit(name: &str) -> &'static str {
    if name.ends_with("_hz") {
        "Hz"
    } else if name.ends_with("_ms") {
        "ms"
    } else if name.ends_with("_db") {
        "dB"
    } else if name.ends_with("_semitones") {
        "st"
    } else if name.ends_with("_s") {
        "s"
    } else if name == "bpm" {
        "BPM"
    } else {
        ""
    }
}

/// Root application state for the node editor window.
pub struct EditorApp {
    camera: Camera,
    patch: PatchView,
    style: NodeStyle,
    /// Node kinds the palette can spawn.
    registry: Registry,
    /// Live palette search text.
    palette_query: String,
    /// Canvas-space centre of the last painted viewport; new nodes spawn there.
    spawn_at: Pos2,
    /// Node being dragged across the canvas, if any.
    moving: Option<vb_core::NodeId>,
    /// Macro-entry stack: each entry is (the parent patch, the macro node entered).
    /// `patch` is always the ACTIVE patch; parents wait here until we go back up.
    stack: Vec<(PatchView, vb_core::NodeId)>,
    /// Snap node positions to the grid when a drag ends.
    snap_to_grid: bool,
    /// Active colour palette for the canvas and inspector widgets.
    theme: Theme,
    /// Set once per session so the egui base visuals are installed on the first frame
    /// and again whenever the theme is toggled.
    visuals_dirty: bool,
    /// Directory reusable macro modules are exported to and imported from.
    modules_dir: PathBuf,
    /// Searchable, tag-filterable browser over the macro modules in [`Self::modules_dir`]
    /// (backlog 000097). Its items are reloaded from disk by [`Self::refresh_library`]; the
    /// query and tag facets it holds persist across frames.
    library: LibraryBrowser,
    /// Whether the library has been loaded from disk yet. The first render of the library
    /// section loads it once (keeping construction filesystem-free); the refresh button and
    /// a successful macro export reload it thereafter.
    library_loaded: bool,
    /// Last export/import outcome, shown in the toolbar until the next one.
    module_status: Option<String>,
    /// Shared RON text buffer for whole-project import/export (000082). Export writes the
    /// current patch here; import parses whatever is in it. Filesystem-free, so it works
    /// in the browser as well as natively.
    project_ron: String,
    /// Last whole-project import/export outcome, shown next to the buffer.
    project_status: Option<String>,
    /// Which surface is shown: the node graph (Blocks) or the control panel (Panel).
    view_mode: ViewMode,
    /// The native AI assistant panel and its live edit session (backlog 000108). Behind the
    /// `assistant` feature so the provider/HTTP stack never reaches the wasm or plugin builds.
    #[cfg(feature = "assistant")]
    assistant: AssistantHost,
}

/// The editor's two surfaces. **Structure** is the node/wire graph where a patch is built;
/// **Panel** is an auto-generated control surface — one live-bound widget per node parameter —
/// the performance-facing view a player tweaks without seeing the wiring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// The node graph (blocks and wires).
    #[default]
    Structure,
    /// The auto-generated control panel.
    Panel,
}

/// One control on the [`ViewMode::Panel`] surface: a node parameter to show and edit, resolved
/// from the live patch. Returned by [`EditorApp::panel_widgets`] so the binding set is testable
/// independently of rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelBinding {
    /// The node the parameter belongs to.
    pub node: vb_core::NodeId,
    /// The parameter's index within that node.
    pub index: usize,
    /// Display label — `"<kind> <param>"`, e.g. `"ladder base_hz"`.
    pub label: String,
    /// Current value.
    pub value: f32,
    /// Range bounds.
    pub min: f32,
    pub max: f32,
}

impl Default for EditorApp {
    fn default() -> Self {
        let registry = Registry::with_builtins();
        Self {
            camera: Camera::default(),
            patch: demo_patch(&registry),
            style: NodeStyle::default(),
            registry,
            palette_query: String::new(),
            spawn_at: Pos2::new(80.0, 220.0),
            moving: None,
            stack: Vec::new(),
            snap_to_grid: true,
            theme: Theme::default(),
            visuals_dirty: true,
            modules_dir: PathBuf::from("modules"),
            // Populated from the modules dir on the first frame (empty when it does not exist
            // yet); the user reloads it with the library section's refresh button.
            library: LibraryBrowser::default(),
            library_loaded: false,
            module_status: None,
            project_ron: String::new(),
            project_status: None,
            view_mode: ViewMode::default(),
            #[cfg(feature = "assistant")]
            assistant: AssistantHost::new(),
        }
    }
}

/// Starter content so the canvas isn't empty: the **Moog Minitaur** voice the plugin also
/// ships as its default (000164), loaded from the committed `examples/minitaur.ron` (generated
/// from `vb_engine::minitaur_patch`) and auto-laid-out by [`PatchView::apply_doc`]. Keeping the
/// editor's starting patch and the plugin default the *same* voice is what the user asked for;
/// it is note-gated, so the standalone is silent until a note plays.
fn demo_patch(registry: &Registry) -> PatchView {
    const MINITAUR_RON: &str = include_str!("../../../examples/minitaur.ron");
    let mut patch = PatchView::new();
    if let Ok(doc) = vb_project::from_ron(MINITAUR_RON) {
        patch.apply_doc(&doc, registry);
    }
    // Seeding is not a user edit: undo must not walk back into an empty canvas.
    patch.clear_history();
    patch
}

/// The old hand-built demo, kept only for the wiring/param tests that need a small,
/// known-shape patch (osc → filter → gain) independent of the shipped Minitaur voice.
#[cfg(test)]
fn sample_wiring_patch(registry: &Registry) -> PatchView {
    let params_of = |kind: &str| registry.get(kind).map(|d| d.params()).unwrap_or_default();
    let mut patch = PatchView::new();
    let osc = patch.add_node_with_params(
        GraphNode::new(
            "oscillator",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ),
        params_of("oscillator"),
        Pos2::new(0.0, 0.0),
    );
    let flt = patch.add_node_with_params(
        GraphNode::new(
            "biquad",
            vec![
                Port::input("in", SignalType::Audio),
                Port::input("cutoff", SignalType::Control),
            ],
            vec![Port::output("out", SignalType::Audio)],
        ),
        params_of("biquad"),
        Pos2::new(180.0, 10.0),
    );
    // A VCA gates the voice: the audio passes through it, and it is shut at rest, so the
    // synth is SILENT the instant it loads instead of droning a held tone. An envelope
    // driven by note events opens it (see below). This mirrors the plugin's gated
    // `vb_engine::default_patch`; keeping the two in sync is what kept the standalone
    // humming after the plugin was fixed.
    let vca = patch.add_node_with_params(
        GraphNode::new(
            "vca",
            vec![
                Port::input("in", SignalType::Audio),
                Port::input("cv", SignalType::Control),
            ],
            vec![Port::output("out", SignalType::Audio)],
        ),
        params_of("vca"),
        Pos2::new(360.0, 10.0),
    );
    let amp = patch.add_node_with_params(
        GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ),
        params_of("gain"),
        Pos2::new(540.0, 20.0),
    );
    // Note gate → envelope → VCA control: a held note fades the voice in, release fades out.
    let midi = patch.add_node_with_params(
        GraphNode::new(
            "midi-in",
            vec![],
            vec![
                Port::output("gate", SignalType::Control),
                Port::output("pitch_hz", SignalType::Control),
                Port::output("velocity", SignalType::Control),
            ],
        ),
        params_of("midi-in"),
        Pos2::new(60.0, 200.0),
    );
    let env = patch.add_node_with_params(
        GraphNode::new(
            "adsr",
            vec![Port::input("gate", SignalType::Control)],
            vec![Port::output("level", SignalType::Control)],
        ),
        params_of("adsr"),
        Pos2::new(260.0, 200.0),
    );
    // Audio path: oscillator → filter → VCA → gain.
    patch
        .connect(osc, PortId(0), flt, PortId(0))
        .expect("demo osc→filter");
    patch
        .connect(flt, PortId(0), vca, PortId(0))
        .expect("demo filter→vca");
    patch
        .connect(vca, PortId(0), amp, PortId(0))
        .expect("demo vca→gain");
    // Control path: note gate → envelope → VCA cv.
    patch
        .connect(midi, PortId(0), env, PortId(0))
        .expect("demo midi→adsr");
    patch
        .connect(env, PortId(0), vca, PortId(1))
        .expect("demo adsr→vca.cv");
    // Shut the VCA at rest: the registry default bias (1.0) is always-open, which would
    // drone; 0.0 means "closed unless the envelope's cv opens it".
    if let Some(i) = patch
        .params(vca)
        .iter()
        .position(|s| s.param.name == "bias")
    {
        patch.set_param(vca, i, 0.0);
    }
    // Ears-friendly starting level: the registry default (1.0) is full scale.
    let gain_index = patch
        .params(amp)
        .iter()
        .position(|s| s.param.name == "gain");
    if let Some(i) = gain_index {
        patch.set_param(amp, i, 0.2);
    }
    // Seeding is not a user edit: undo must not walk back into an empty canvas.
    patch.clear_history();
    patch
}

/// Draws one wire as a horizontal-tangent cubic Bézier between two screen points.
fn draw_wire(painter: &egui::Painter, from: Pos2, to: Pos2, width: f32, color: Color32) {
    let bend = ((to.x - from.x).abs() * 0.5).max(24.0);
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
        [
            from,
            from + Vec2::new(bend, 0.0),
            to - Vec2::new(bend, 0.0),
            to,
        ],
        false,
        Color32::TRANSPARENT,
        Stroke::new(width, color),
    ));
}

/// Draws a rotary knob for `value` within `param`'s range and returns its response
/// plus, when a drag moved it, the new value (already inside range).
///
/// The knob reads `[min, max]` over a 270° arc with the gap at the bottom; vertical
/// drag turns it via [`widgets::knob_drag`]. Colours come from `theme`.
fn knob_widget(
    ui: &mut Ui,
    theme: &Theme,
    param: &Parameter,
    value: f32,
) -> (egui::Response, Option<f32>) {
    const DIAMETER: f32 = 44.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(DIAMETER), Sense::drag());
    let norm = param.normalize(value);

    let changed = response.dragged().then(|| {
        let new_norm = widgets::knob_drag(norm, response.drag_delta().y);
        param.denormalize(new_norm)
    });

    let painter = ui.painter_at(rect);
    let center = rect.center();
    let radius = DIAMETER * 0.5 - 4.0;
    // 225° (down-left) sweeping clockwise to −45° (down-right): a bottom-gapped dial.
    let at = |t: f32| {
        let a = (225.0 - 270.0 * t).to_radians();
        Pos2::new(center.x + radius * a.cos(), center.y - radius * a.sin())
    };
    let track: Vec<Pos2> = (0..=32).map(|k| at(k as f32 / 32.0)).collect();
    painter.add(egui::Shape::line(
        track,
        Stroke::new(3.0_f32, theme.node_outline),
    ));
    if norm > 0.0 {
        let upto = (32.0 * norm).ceil() as usize;
        let value_arc: Vec<Pos2> = (0..=upto)
            .map(|k| at((k as f32 / 32.0).min(norm)))
            .collect();
        painter.add(egui::Shape::line(
            value_arc,
            Stroke::new(3.0_f32, theme.accent),
        ));
    }
    let tip = at(norm);
    painter.line_segment([center, tip], Stroke::new(2.0_f32, theme.text));
    painter.circle_filled(tip, 3.0, theme.text);
    (response, changed)
}

/// Draws a stepped selector for `value` and returns the new value when a step is taken.
///
/// The `◀`/`▶` buttons cycle the parameter's discrete positions (an enum's members, or
/// a two-state min/max toggle) via [`widgets::switch_step`], wrapping at the ends.
fn switch_widget(ui: &mut Ui, param: &Parameter, value: f32) -> Option<f32> {
    let values = switch_values(param);
    let idx = switch_index(param, value);
    let mut out = None;
    ui.horizontal(|ui| {
        if ui.small_button("◀").clicked() {
            out = Some(switch_step(param, value, false));
        }
        ui.monospace(format!("{}/{}", idx + 1, values.len()));
        ui.weak(format!("{:.3}", values[idx]));
        if ui.small_button("▶").clicked() {
            out = Some(switch_step(param, value, true));
        }
    });
    out
}

/// Draws a 2-D pad binding `x_param` (horizontal) to `y_param` (vertical) and returns
/// the new `(x_value, y_value)` while the pointer drags or clicks inside it.
///
/// The handle sits at [`xy_norm_to_pad`] of the two normalized values; a grab reads
/// back through [`xy_pad_to_norm`] (Y inverted, so up = maximum).
fn xy_pad_widget(
    ui: &mut Ui,
    theme: &Theme,
    x_param: &Parameter,
    x_value: f32,
    y_param: &Parameter,
    y_value: f32,
) -> Option<(f32, f32)> {
    const SIDE: f32 = 116.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SIDE), Sense::click_and_drag());

    let out = (response.dragged() || response.clicked())
        .then(|| response.interact_pointer_pos())
        .flatten()
        .map(|p| {
            let local = (p - rect.min) / SIDE;
            let (xn, yn) = xy_pad_to_norm(local.x, local.y);
            (x_param.denormalize(xn), y_param.denormalize(yn))
        });

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme.canvas_bg);
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0_f32, theme.node_outline),
        egui::StrokeKind::Inside,
    );
    // Cross-hairs through the current handle position.
    let (hx, hy) = xy_norm_to_pad(x_param.normalize(x_value), y_param.normalize(y_value));
    let handle = rect.min + Vec2::new(hx * SIDE, hy * SIDE);
    let faint = Stroke::new(1.0_f32, theme.grid_line_strong);
    painter.line_segment(
        [
            Pos2::new(rect.min.x, handle.y),
            Pos2::new(rect.max.x, handle.y),
        ],
        faint,
    );
    painter.line_segment(
        [
            Pos2::new(handle.x, rect.min.y),
            Pos2::new(handle.x, rect.max.y),
        ],
        faint,
    );
    painter.circle_filled(handle, 6.0, theme.accent);
    painter.circle_stroke(handle, 6.0, Stroke::new(1.0_f32, theme.canvas_bg));
    out
}

impl EditorApp {
    /// Creates the app for `eframe::run_native`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The camera the canvas is currently viewed through.
    #[must_use]
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// The patch shown on the canvas.
    #[must_use]
    pub fn patch(&self) -> &PatchView {
        &self.patch
    }

    /// Mutable patch access for integration code (standalone, tests).
    pub fn patch_mut(&mut self) -> &mut PatchView {
        &mut self.patch
    }

    /// Applies an assistant-proposed [`ProjectDoc`] to the active patch as one undoable
    /// command, using this app's registry to seed parameters. Returns `true` if applied
    /// (a valid document); an invalid one is refused. See [`PatchView::apply_doc`].
    pub fn apply_doc(&mut self, doc: &vb_project::ProjectDoc) -> bool {
        self.patch.apply_doc(doc, &self.registry)
    }

    /// Draws the canvas into `ui` and applies pan/zoom/select input.
    ///
    /// Split out of [`eframe::App::ui`] so integration code (and later the
    /// standalone) can embed the canvas in any container.
    pub fn canvas_ui(&mut self, ui: &mut Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let viewport = response.rect;
        let origin = viewport.min.to_vec2();
        // Palette spawns land in the middle of whatever is currently visible.
        self.spawn_at = self.camera.screen_to_canvas(viewport.center() - origin);

        // A drag starting on a port pulls a wire; on a node's title/body it moves
        // that node; anywhere else it pans.
        if response.drag_started() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let canvas = self.camera.screen_to_canvas(pointer - origin);
                if self.patch.begin_wire(canvas, &self.style).is_none() {
                    if let Some((id, _)) = self.patch.node_at(canvas, &self.style) {
                        self.patch.click(canvas, &self.style); // select + raise
                        self.moving = Some(id);
                    }
                }
            }
        }
        if response.dragged() && self.patch.wire_drag().is_none() {
            match self.moving {
                // Screen-pixel drag ÷ zoom = canvas delta, so the node tracks the
                // pointer 1:1 at any zoom.
                Some(id) => {
                    let delta = response.drag_delta() / self.camera.zoom();
                    self.patch.move_node(id, delta);
                }
                None => self.camera.pan_by(response.drag_delta()),
            }
        }
        if response.drag_stopped() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let canvas = self.camera.screen_to_canvas(pointer - origin);
                self.patch.end_wire(canvas, &self.style);
            }
            if let (Some(id), true) = (self.moving, self.snap_to_grid) {
                // Snap to the base grid spacing so nodes line up across zoom levels.
                self.patch.snap_node(id, 32.0);
            }
            self.moving = None;
            self.patch.end_gesture();
        }

        // Keyboard: delete/duplicate the selection — unless a text field (e.g. the
        // palette search box) owns the keyboard.
        if !ui.ctx().wants_keyboard_input() {
            let (delete, duplicate, undo, redo) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                    i.modifiers.command && i.key_pressed(egui::Key::D),
                    i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z),
                )
            });
            if delete {
                self.patch.remove_selected();
            }
            if duplicate {
                self.patch.duplicate_selected();
            }
            if undo {
                self.patch.undo();
            }
            if redo {
                self.patch.redo();
            }
        }

        // Zoom: scroll/pinch, anchored at the pointer.
        if let Some(pointer) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let pinch = ui.input(|i| i.zoom_delta());
            let factor = pinch * (scroll * 0.003).exp();
            if (factor - 1.0).abs() > f32::EPSILON {
                // The camera space has its origin at the viewport corner.
                self.camera.zoom_at(pointer - origin, factor);
            }
        }

        // Select: click through to the topmost node, or clear on empty canvas.
        if response.clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let canvas = self.camera.screen_to_canvas(pointer - origin);
                self.patch.click(canvas, &self.style);
            }
        }

        // Double-click a macro node to step inside; Escape steps back out.
        if response.double_clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let canvas = self.camera.screen_to_canvas(pointer - origin);
                if let Some((id, _)) = self.patch.node_at(canvas, &self.style) {
                    self.enter_macro(id);
                }
            }
        }
        if !ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.leave_macro();
        }

        self.paint_grid(&painter, viewport);
        self.paint_wires(&painter, origin, response.hover_pos());
        self.paint_nodes(&painter, origin);
    }

    /// Paints every graph edge as a Bézier wire, plus the pending wire while dragging.
    fn paint_wires(&self, painter: &egui::Painter, origin: Vec2, pointer: Option<Pos2>) {
        let to_screen = |p: Pos2| self.camera.canvas_to_screen(p) + origin;
        let stroke_w = (2.0 * self.camera.zoom()).clamp(1.0, 4.0);

        for edge in self.patch.graph().edges() {
            let (Some(from), Some(to)) = (
                self.patch.port_anchor(
                    edge.from_node,
                    PortDirection::Output,
                    edge.from_port,
                    &self.style,
                ),
                self.patch.port_anchor(
                    edge.to_node,
                    PortDirection::Input,
                    edge.to_port,
                    &self.style,
                ),
            ) else {
                continue;
            };
            let color = self
                .patch
                .graph()
                .node(edge.from_node)
                .and_then(|n| n.outputs.get(edge.from_port.0))
                .map_or(self.theme.text_dim, |p| self.theme.signal_color(p.signal));
            draw_wire(painter, to_screen(from), to_screen(to), stroke_w, color);
        }

        // The pending wire follows the pointer from the grabbed port, tinted by
        // whether the port under the pointer would accept the drop.
        if let (Some(drag), Some(pointer)) = (self.patch.wire_drag(), pointer) {
            if let Some(anchor) =
                self.patch
                    .port_anchor(drag.node, drag.direction, drag.port, &self.style)
            {
                let (from, to) = match drag.direction {
                    PortDirection::Output => (to_screen(anchor), pointer),
                    PortDirection::Input => (pointer, to_screen(anchor)),
                };
                let color = match self.drop_target_ok(pointer - origin) {
                    Some(true) => Color32::from_rgb(0x4c, 0xd9, 0x64),
                    Some(false) => Color32::from_rgb(0xe5, 0x48, 0x48),
                    None => self.theme.text_dim,
                };
                draw_wire(painter, from, to, stroke_w, color);
            }
        }
    }

    /// Validity of the port under a viewport-local pointer as a drop target for the
    /// active wire drag: `None` when not over a port (or no drag).
    fn drop_target_ok(&self, screen: Pos2) -> Option<bool> {
        let canvas = self.camera.screen_to_canvas(screen);
        match self.patch.node_at(canvas, &self.style)? {
            (node, crate::layout::NodeHit::Input(port)) => {
                self.patch.wire_target_ok(node, PortDirection::Input, port)
            }
            (node, crate::layout::NodeHit::Output(port)) => {
                self.patch.wire_target_ok(node, PortDirection::Output, port)
            }
            _ => None,
        }
    }

    /// Paints the adaptive background grid over `viewport`. Every fourth line is drawn
    /// a touch stronger, giving the eye major gridlines to read spacing against.
    fn paint_grid(&self, painter: &egui::Painter, viewport: Rect) {
        painter.rect_filled(viewport, 0.0, self.theme.canvas_bg);

        let step = grid_step(self.camera.zoom());
        let minor = Stroke::new(1.0_f32, self.theme.grid_line);
        let major = Stroke::new(1.0_f32, self.theme.grid_line_strong);
        let origin = viewport.min.to_vec2();

        // Canvas-space bounds of the visible viewport.
        let top_left = self.camera.screen_to_canvas(Pos2::ZERO);
        let bottom_right = self
            .camera
            .screen_to_canvas((viewport.max - viewport.min).to_pos2());

        // Index a line by how many `step`s it sits from the canvas origin, so the major
        // lines stay pinned to the same canvas positions as the view pans.
        let stroke_for = |line_index: f32| {
            if (line_index.rem_euclid(4.0)).abs() < 0.5 {
                major
            } else {
                minor
            }
        };

        let first_x = (top_left.x / step).floor();
        let mut xi = first_x;
        let mut x = xi * step;
        while x <= bottom_right.x {
            let sx = self.camera.canvas_to_screen(Pos2::new(x, 0.0)).x + origin.x;
            painter.vline(sx, viewport.y_range(), stroke_for(xi));
            xi += 1.0;
            x = xi * step;
        }
        let first_y = (top_left.y / step).floor();
        let mut yi = first_y;
        let mut y = yi * step;
        while y <= bottom_right.y {
            let sy = self.camera.canvas_to_screen(Pos2::new(0.0, y)).y + origin.y;
            painter.hline(viewport.x_range(), sy, stroke_for(yi));
            yi += 1.0;
            y = yi * step;
        }
    }

    /// Paints every node in draw order through the camera.
    fn paint_nodes(&self, painter: &egui::Painter, origin: Vec2) {
        let zoom = self.camera.zoom();
        let to_screen = |p: Pos2| self.camera.canvas_to_screen(p) + origin;

        for (id, node, pos) in self.patch.draw_order() {
            let l: NodeLayout = layout_node(node, pos, &self.style);
            let frame = Rect::from_min_max(to_screen(l.frame.min), to_screen(l.frame.max));
            let title = Rect::from_min_max(to_screen(l.title.min), to_screen(l.title.max));
            let selected = self.patch.selected() == Some(id);

            let rounding =
                CornerRadius::same((4.0 * zoom).clamp(1.0, self.theme.node_rounding) as u8);
            // Soft drop shadow for depth, then the body with a lighter top band that
            // fakes a top-lit gradient, then the title bar over it.
            let shadow = frame.translate(Vec2::splat(2.0 * zoom));
            painter.rect_filled(shadow, rounding, self.theme.node_shadow);
            painter.rect_filled(frame, rounding, self.theme.node_fill);
            let highlight = Rect::from_min_max(
                frame.min,
                Pos2::new(frame.max.x, frame.min.y + frame.height() * 0.45),
            );
            painter.rect_filled(highlight, rounding, self.theme.node_fill_top);
            painter.rect_filled(title, rounding, self.theme.node_title);

            if selected {
                // A faint outer halo under the crisp outline reads as a glow.
                painter.rect_stroke(
                    frame.expand(2.5 * zoom),
                    rounding,
                    Stroke::new(
                        2.0_f32,
                        self.theme.node_outline_selected.gamma_multiply(0.4),
                    ),
                    egui::StrokeKind::Outside,
                );
            }
            let outline = if selected {
                Stroke::new(2.0_f32, self.theme.node_outline_selected)
            } else {
                Stroke::new(1.0_f32, self.theme.node_outline)
            };
            painter.rect_stroke(frame, rounding, outline, egui::StrokeKind::Outside);

            painter.text(
                title.center(),
                Align2::CENTER_CENTER,
                &node.kind,
                FontId::proportional((12.0 * zoom).clamp(8.0, 24.0)),
                self.theme.text,
            );

            let port_r = self.style.port_radius * zoom;
            let label_font = FontId::proportional((10.0 * zoom).clamp(7.0, 20.0));
            for (i, (anchor, port, align, direction)) in l
                .inputs
                .iter()
                .zip(&node.inputs)
                .enumerate()
                .map(|(i, (a, p))| (i, (a, p, Align2::LEFT_CENTER, PortDirection::Input)))
                .chain(
                    l.outputs
                        .iter()
                        .zip(&node.outputs)
                        .enumerate()
                        .map(|(i, (a, p))| {
                            (i, (a, p, Align2::RIGHT_CENTER, PortDirection::Output))
                        }),
                )
            {
                let at = to_screen(*anchor);
                // While a wire is dragged, valid drop targets get a halo and invalid
                // ports fade out; the grabbed port keeps its normal look.
                let target_ok = self.patch.wire_target_ok(id, direction, PortId(i));
                let is_grabbed = self.patch.wire_drag().is_some_and(|d| {
                    d.node == id && d.direction == direction && d.port == PortId(i)
                });
                let mut fill = self.theme.signal_color(port.signal);
                if target_ok == Some(false) && !is_grabbed {
                    fill = fill.gamma_multiply(0.3);
                }
                if target_ok == Some(true) {
                    painter.circle_stroke(
                        at,
                        port_r + (3.0 * zoom).clamp(2.0, 6.0),
                        Stroke::new(2.0_f32, Color32::from_rgb(0x4c, 0xd9, 0x64)),
                    );
                }
                painter.circle_filled(at, port_r, fill);
                painter.circle_stroke(at, port_r, Stroke::new(1.0_f32, self.theme.canvas_bg));
                // Port names clutter at far zoom-out; show them once readable.
                if zoom >= 0.6 {
                    let inset = Vec2::new(
                        if align == Align2::LEFT_CENTER {
                            port_r + 3.0
                        } else {
                            -(port_r + 3.0)
                        },
                        0.0,
                    );
                    painter.text(
                        at + inset,
                        align,
                        &port.name,
                        label_font.clone(),
                        self.theme.text_dim,
                    );
                }
            }
        }
    }

    /// Test/integration hook: pans the camera as a drag by `delta` screen pixels would.
    pub fn pan(&mut self, delta: Vec2) {
        self.camera.pan_by(delta);
    }

    /// Test/integration hook: zooms as a scroll at `anchor` (viewport-local) would.
    pub fn zoom(&mut self, anchor: Pos2, factor: f32) {
        self.camera.zoom_at(anchor, factor);
    }

    /// How deep inside macros the editor currently is (0 = root).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Enters macro `id` of the active patch: its subpatch becomes the active patch.
    /// Returns `false` if `id` is not a macro.
    pub fn enter_macro(&mut self, id: vb_core::NodeId) -> bool {
        let Some(sub) = self.patch.take_subpatch(id) else {
            return false;
        };
        let parent = std::mem::replace(&mut self.patch, sub);
        self.stack.push((parent, id));
        self.moving = None;
        true
    }

    /// Goes one level up, returning the edited subpatch to its macro node.
    /// Returns `false` at the root.
    pub fn leave_macro(&mut self) -> bool {
        let Some((mut parent, id)) = self.stack.pop() else {
            return false;
        };
        let sub = std::mem::replace(&mut self.patch, PatchView::new());
        parent.restore_subpatch(id, sub);
        self.patch = parent;
        self.moving = None;
        true
    }

    /// Test/integration hook: clicks at a viewport-local screen position.
    pub fn click(&mut self, screen: Pos2) {
        let canvas = self.camera.screen_to_canvas(screen);
        self.patch.click(canvas, &self.style);
    }

    /// Spawns a registry node of `kind` at the centre of the visible canvas and
    /// selects it. Returns the new node's id, or `None` for an unknown kind.
    pub fn spawn_node(&mut self, kind: &str) -> Option<vb_core::NodeId> {
        let descriptor = self.registry.get(kind)?;
        let node = descriptor.graph_node();
        let params = descriptor.params();
        // Centre the node's frame on the spawn point.
        let half = Vec2::new(self.style.width * 0.5, self.style.title_height);
        let id = self
            .patch
            .add_node_with_params(node, params, self.spawn_at - half);
        self.patch
            .click(self.spawn_at - half + Vec2::new(4.0, 4.0), &self.style);
        Some(id)
    }

    /// Renders the inspector panel: name and editable parameters of the selection.
    fn inspector_ui(&mut self, ui: &mut Ui) {
        ui.heading("inspector");
        let Some(id) = self.patch.selected() else {
            ui.weak("nothing selected");
            return;
        };
        let kind = self
            .patch
            .graph()
            .node(id)
            .map_or_else(|| "?".to_owned(), |n| n.kind.clone());
        ui.label(kind);
        ui.separator();

        if self.patch.params(id).is_empty() {
            ui.weak("no parameters");
            return;
        }
        // Copy the slots out so the widgets can borrow `ui` while edits go back through
        // set_param (which re-clamps) and set_widget.
        let slots: Vec<_> = self.patch.params(id).to_vec();
        let mut i = 0;
        while i < slots.len() {
            i += self.param_widget_ui(ui, id, &slots, i);
            ui.add_space(6.0);
        }
    }

    /// Draws one parameter's control (chosen by its [`WidgetKind`]) with a widget-type
    /// picker above it. Returns how many slots it consumed — an XY-pad claims two, every
    /// other widget one — so the inspector loop can advance past a paired parameter.
    fn param_widget_ui(
        &mut self,
        ui: &mut Ui,
        id: vb_core::NodeId,
        slots: &[crate::patch::ParamSlot],
        i: usize,
    ) -> usize {
        let slot = &slots[i];
        let theme = self.theme; // Copy — frees `self` for the set_param calls below.

        // Header: parameter name (+ unit) + its range + the widget-type picker, so a knob
        // reads as "what it controls, in what unit, over what range" rather than a bare dial.
        let mut widget = slot.widget;
        ui.horizontal(|ui| {
            let unit = param_unit(&slot.param.name);
            if unit.is_empty() {
                ui.strong(&slot.param.name);
            } else {
                ui.strong(format!("{} ({unit})", slot.param.name));
            }
            ui.weak(format!("[{}–{}]", slot.param.min, slot.param.max));
            egui::ComboBox::from_id_salt((id, i, "widget"))
                .width(72.0)
                .selected_text(widget.label())
                .show_ui(ui, |ui| {
                    for kind in WidgetKind::ALL {
                        ui.selectable_value(&mut widget, kind, kind.label());
                    }
                });
        });
        if widget != slot.widget {
            self.patch.set_widget(id, i, widget);
        }

        match widget {
            WidgetKind::Slider => {
                let mut value = slot.value;
                let range = slot.param.min..=slot.param.max;
                let response = ui.add(egui::Slider::new(&mut value, range));
                if response.changed() {
                    self.patch.set_param(id, i, value);
                }
                if response.drag_stopped() || response.lost_focus() {
                    self.patch.end_gesture();
                }
                1
            }
            WidgetKind::Knob => {
                let (response, changed) = knob_widget(ui, &theme, &slot.param, slot.value);
                if let Some(v) = changed {
                    self.patch.set_param(id, i, v);
                }
                if response.drag_stopped() {
                    self.patch.end_gesture();
                }
                ui.weak(format!("{:.3}", self.patch.params(id)[i].value));
                1
            }
            WidgetKind::Switch => {
                if let Some(v) = switch_widget(ui, &slot.param, slot.value) {
                    self.patch.set_param(id, i, v);
                    self.patch.end_gesture();
                }
                1
            }
            WidgetKind::XyPad => {
                // Pair with the next parameter as the Y axis; a lone last parameter has
                // no partner, so fall back to a knob for it.
                let Some(y_slot) = slots.get(i + 1) else {
                    let (response, changed) = knob_widget(ui, &theme, &slot.param, slot.value);
                    if let Some(v) = changed {
                        self.patch.set_param(id, i, v);
                    }
                    if response.drag_stopped() {
                        self.patch.end_gesture();
                    }
                    ui.weak("(xy needs a second parameter)");
                    return 1;
                };
                ui.weak(format!("x: {} · y: {}", slot.param.name, y_slot.param.name));
                if let Some((xv, yv)) = xy_pad_widget(
                    ui,
                    &theme,
                    &slot.param,
                    slot.value,
                    &y_slot.param,
                    y_slot.value,
                ) {
                    self.patch.set_param(id, i, xv);
                    self.patch.set_param(id, i + 1, yv);
                    self.patch.end_gesture();
                }
                2
            }
        }
    }

    /// Renders the palette panel contents: search box + one button per matching kind,
    /// grouped by category.
    fn palette_ui(&mut self, ui: &mut Ui) {
        ui.heading("nodes");
        ui.text_edit_singleline(&mut self.palette_query);
        if ui.button("+ macro").clicked() {
            let half = Vec2::new(self.style.width * 0.5, self.style.title_height);
            self.patch.add_macro(self.spawn_at - half);
        }
        self.factory_ui(ui);
        self.modules_ui(ui);
        self.library_ui(ui);
        #[cfg(feature = "assistant")]
        self.assistant_ui(ui);
        ui.separator();
        self.project_io_ui(ui);
        ui.separator();
        let entries = palette_entries(&self.registry, &self.palette_query);
        if entries.is_empty() {
            ui.weak("no matches");
            return;
        }
        let mut spawn: Option<&'static str> = None;
        let mut last_category = "";
        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in &entries {
                if entry.category != last_category {
                    ui.add_space(4.0);
                    ui.weak(entry.category);
                    last_category = entry.category;
                }
                if ui.button(entry.kind).clicked() {
                    spawn = Some(entry.kind);
                }
            }
        });
        if let Some(kind) = spawn {
            self.spawn_node(kind);
        }
    }

    /// Whole-project RON import/export (000082): a text buffer plus Export/Import buttons.
    /// Filesystem-free, so the same controls work in the browser — copy the RON out, or
    /// paste a `.vbp` in and load it.
    fn project_io_ui(&mut self, ui: &mut Ui) {
        ui.label("project (RON)");
        ui.horizontal(|ui| {
            if ui.button("export ⤓").clicked() {
                self.project_status = Some(match self.export_project_ron() {
                    Ok(ron) => {
                        self.project_ron = ron;
                        "exported to the buffer".to_owned()
                    }
                    Err(e) => format!("export failed: {e}"),
                });
            }
            let can_import = !self.project_ron.trim().is_empty();
            if ui
                .add_enabled(can_import, egui::Button::new("import ⤒"))
                .clicked()
            {
                let text = self.project_ron.clone();
                self.project_status = Some(match self.import_project_ron(&text) {
                    Ok(()) => "loaded project".to_owned(),
                    Err(e) => format!("import failed: {e}"),
                });
            }
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.project_ron)
                .desired_rows(4)
                .code_editor()
                .hint_text("exported RON appears here; paste a .vbp to import"),
        );
        if let Some(status) = &self.project_status {
            ui.weak(status);
        }
    }

    /// Reusable-module controls: export the selected macro, and import any module saved
    /// under [`Self::modules_dir`] as a fresh macro node.
    fn modules_ui(&mut self, ui: &mut Ui) {
        let sel_macro = self.patch.selected().filter(|id| self.patch.is_macro(*id));
        if ui
            .add_enabled(sel_macro.is_some(), egui::Button::new("export macro"))
            .clicked()
        {
            if let Some(id) = sel_macro {
                let dir = self.modules_dir.clone();
                self.module_status = Some(match self.export_macro_to_dir(id, &dir) {
                    Ok(path) => {
                        // The new module should show up in the library immediately.
                        self.refresh_library();
                        format!("exported → {}", path.display())
                    }
                    Err(e) => format!("export failed: {e}"),
                });
            }
        }

        let modules = list_modules(&self.modules_dir);
        if !modules.is_empty() {
            let mut chosen: Option<PathBuf> = None;
            egui::ComboBox::from_id_salt("import-module")
                .selected_text("import module ▾")
                .show_ui(ui, |ui| {
                    for path in &modules {
                        let label = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        if ui.selectable_label(false, label).clicked() {
                            chosen = Some(path.clone());
                        }
                    }
                });
            if let Some(path) = chosen {
                self.module_status = Some(match self.import_macro_from_file(&path) {
                    Ok(id) => format!("imported macro#{}", id.0),
                    Err(e) => format!("import failed: {e}"),
                });
            }
        }
        if let Some(status) = &self.module_status {
            ui.weak(status);
        }
    }

    /// Reloads the library's items from [`Self::modules_dir`], keeping the current search
    /// query and tag facets. Called on the first library render, by the refresh button, and
    /// after a macro is exported (so the new module appears).
    pub fn refresh_library(&mut self) {
        self.library
            .set_items(library_items_from_module_dir(&self.modules_dir));
        self.library_loaded = true;
    }

    /// Imports the saved module whose name is `name` as a new macro node, resolving the name
    /// to its file by scanning [`Self::modules_dir`]. The library lists modules by name, but
    /// the file stem is sanitized (and so not reversible), so the name is matched against
    /// each module's stored `name`; the first match wins.
    ///
    /// # Errors
    /// [`ModuleError::Io`] if no saved module carries `name`, or any [`ModuleError`] from
    /// reading/rebuilding the matched module.
    pub fn import_module_by_name(&mut self, name: &str) -> Result<vb_core::NodeId, ModuleError> {
        let path = list_modules(&self.modules_dir)
            .into_iter()
            .find(|path| MacroModule::read_from_file(path).is_ok_and(|module| module.name == name));
        match path {
            Some(path) => self.import_macro_from_file(&path),
            None => Err(ModuleError::Io(format!(
                "no module named {name:?} in the library"
            ))),
        }
    }

    /// Replaces the whole canvas with the factory instrument patch named `name` (000165).
    /// Returns `false` for an unknown name or a patch that fails to load (canvas unchanged).
    /// The load carries no undo history, like [`Self::load_doc`].
    pub fn load_factory_patch(&mut self, name: &str) -> bool {
        let Some(patch) = crate::factory::factory_patch(name) else {
            return false;
        };
        match vb_project::from_ron(patch.ron) {
            Ok(doc) => self.load_doc(&doc),
            Err(_) => false,
        }
    }

    /// Renders the "factory patches" section: one button per built-in instrument (Minitaur,
    /// Model D, Neutron, Edge, …). Clicking one loads it onto the canvas (replacing the
    /// current patch), so a user can recall a recognisable starting point (000165).
    fn factory_ui(&mut self, ui: &mut Ui) {
        ui.collapsing("factory patches", |ui| {
            let mut chosen: Option<&'static str> = None;
            for patch in crate::factory::FACTORY_PATCHES {
                if ui.button(patch.name).clicked() {
                    chosen = Some(patch.name);
                }
            }
            if let Some(name) = chosen {
                self.module_status = Some(if self.load_factory_patch(name) {
                    format!("loaded {name}")
                } else {
                    format!("failed to load {name}")
                });
            }
        });
    }

    /// Renders the library section: a searchable, tag-filtered list of saved macro modules
    /// (backlog 000097). Clicking an entry imports it onto the canvas. Loads the library on
    /// first display and offers a refresh button to reload it from disk.
    fn library_ui(&mut self, ui: &mut Ui) {
        if !self.library_loaded {
            self.refresh_library();
        }
        ui.collapsing("library", |ui| {
            if ui.button("⟳ refresh").clicked() {
                self.refresh_library();
            }
            if let Some(name) = library_browser_ui(ui, &mut self.library) {
                self.module_status = Some(match self.import_module_by_name(&name) {
                    Ok(id) => format!("imported {name} → macro#{}", id.0),
                    Err(e) => format!("import failed: {e}"),
                });
            }
        });
    }

    /// Injects the assistant's LLM backend and resets its working copy so the next request
    /// starts from the live patch. `assistant-net` builds call this via
    /// [`Self::set_assistant_anthropic`] / [`Self::set_assistant_local`]; tests inject an
    /// in-process double. The provider must be `Send + Sync` — it runs on a worker thread.
    #[cfg(feature = "assistant")]
    pub fn set_assistant_provider(&mut self, provider: SharedProvider) {
        self.assistant.provider = Some(provider);
        self.assistant.session = None;
        self.assistant.status = None;
    }

    /// Wires the Anthropic (Claude) provider, reading the BYOK key from the OS keychain under
    /// `keychain_service` at call time. `assistant-net` only.
    ///
    /// # Errors
    /// [`vb_ai::AiError`] if the HTTP client cannot be built.
    #[cfg(feature = "assistant-net")]
    pub fn set_assistant_anthropic(
        &mut self,
        keychain_service: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Result<(), vb_ai::AiError> {
        let transport = vb_ai::ReqwestTransport::new(timeout)?;
        let keys = vb_ai::keystore::KeychainStore::new(keychain_service);
        self.set_assistant_provider(Arc::new(vb_ai::AnthropicProvider::new(transport, keys)));
        Ok(())
    }

    /// Wires a local, OpenAI-compatible backend (e.g. Ollama) over HTTP. `assistant-net` only.
    ///
    /// # Errors
    /// [`vb_ai::AiError`] if the HTTP client cannot be built.
    #[cfg(feature = "assistant-net")]
    pub fn set_assistant_local(
        &mut self,
        backend: vb_ai::LocalBackend,
        timeout: std::time::Duration,
    ) -> Result<(), vb_ai::AiError> {
        let transport = vb_ai::ReqwestTransport::new(timeout)?;
        self.set_assistant_provider(Arc::new(vb_ai::LocalProvider::new(transport, backend)));
        Ok(())
    }

    /// Wires an OpenAI-compatible provider (default OpenAI endpoint), reading the BYOK key
    /// from the OS keychain under `keychain_service`. `assistant-net` only.
    ///
    /// # Errors
    /// [`vb_ai::AiError`] if the HTTP client cannot be built.
    #[cfg(feature = "assistant-net")]
    pub fn set_assistant_openai(
        &mut self,
        keychain_service: impl Into<String>,
        timeout: std::time::Duration,
        endpoint: Option<&str>,
    ) -> Result<(), vb_ai::AiError> {
        let transport = vb_ai::ReqwestTransport::new(timeout)?;
        let keys = vb_ai::keystore::KeychainStore::new(keychain_service);
        let mut provider = vb_ai::OpenAiProvider::new(transport, keys);
        if let Some(url) = endpoint {
            provider = provider.with_url(url);
        }
        self.set_assistant_provider(Arc::new(provider));
        Ok(())
    }

    /// Applies the Settings panel: the worker already snapshots `settings.config()` each
    /// request, so this only (re)connects the live provider. Under `assistant-net` it builds
    /// the provider the panel selected, reading the key from the keychain; without it there is
    /// no network provider to connect, so it just notes that.
    #[cfg(feature = "assistant")]
    fn apply_assistant_settings(&mut self) {
        #[cfg(feature = "assistant-net")]
        {
            let timeout = std::time::Duration::from_secs(60);
            let provider = self.assistant.settings.provider();
            let result = match provider {
                vb_ai::ProviderKind::Anthropic => self.set_assistant_anthropic("VST_bomb", timeout),
                vb_ai::ProviderKind::OpenAiCompatible => {
                    let endpoint = self.assistant.settings.endpoint().map(str::to_owned);
                    self.set_assistant_openai("VST_bomb", timeout, endpoint.as_deref())
                }
            };
            self.assistant.status = Some(match result {
                Ok(()) => format!("connected {}", provider.label()),
                Err(e) => format!("connect failed: {e}"),
            });
        }
        #[cfg(not(feature = "assistant-net"))]
        {
            self.assistant.status = Some(
                "settings applied (build with `assistant-net` to connect a provider)".to_owned(),
            );
        }
    }

    /// Consumes a worker's proposal if one has arrived, recording it on the main thread via
    /// [`EditSession::record`]. Returns `true` if a message was consumed. Called every frame
    /// and, in tests, after joining the worker. No-op when no request is in flight.
    #[cfg(feature = "assistant")]
    fn poll_assistant(&mut self) -> bool {
        let msg = match self.assistant.rx.as_ref() {
            Some(rx) => rx.try_recv(),
            None => return false,
        };
        match msg {
            Ok((instruction, result)) => {
                self.assistant.rx = None;
                if let Some(worker) = self.assistant.worker.take() {
                    let _ = worker.join();
                }
                if let Some(session) = self.assistant.session.as_mut() {
                    self.assistant.status = Some(match session.record(&instruction, result) {
                        Ok(()) => match session.pending().map(|o| o.diff.summary()) {
                            Some(summary) if !summary.trim().is_empty() => {
                                format!("proposed:\n{summary}")
                            }
                            _ => "proposed (no changes)".to_owned(),
                        },
                        Err(e) => format!("assistant failed: {e}"),
                    });
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                // The worker dropped its sender without a message — treat as a stopped request.
                self.assistant.rx = None;
                if let Some(worker) = self.assistant.worker.take() {
                    let _ = worker.join();
                }
                self.assistant.status = Some("assistant worker stopped unexpectedly".to_owned());
                true
            }
        }
    }

    /// Renders the assistant section: the chat transcript, a proposed-edit diff preview with
    /// apply/reject, and the instruction input. Seeds the working copy from the live patch on
    /// first display; shows a "thinking…" line while a worker runs; dispatches the panel's
    /// action to [`Self::handle_assistant_action`].
    #[cfg(feature = "assistant")]
    fn assistant_ui(&mut self, ui: &mut Ui) {
        ui.collapsing("assistant", |ui| {
            if self.assistant.session.is_none() {
                self.assistant.session = Some(EditSession::new(self.patch.to_doc()));
            }
            if self.assistant.provider.is_none() {
                ui.colored_label(
                    Color32::from_rgb(0xd0, 0x90, 0x40),
                    "no AI backend configured — build with the `assistant-net` feature",
                );
            }
            if self.assistant.in_flight() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("thinking…");
                });
            }
            let session = self.assistant.session.as_ref().expect("seeded above");
            let action = assistant_panel_ui(ui, session, &mut self.assistant.draft);
            if let Some(status) = &self.assistant.status {
                ui.separator();
                ui.weak(status);
            }
            if let Some(action) = action {
                self.handle_assistant_action(action);
            }

            // The provider/model/privacy/BYOK-key Settings, collapsed by default.
            ui.collapsing("settings", |ui| {
                let settings_action = settings_panel_ui(
                    ui,
                    &mut self.assistant.settings,
                    &mut *self.assistant.keys,
                    &mut self.assistant.key_draft,
                );
                if settings_action == Some(SettingsAction::Apply) {
                    self.apply_assistant_settings();
                }
            });
        });
    }

    /// Drives the edit session from a panel action: `Submit` spawns a worker that proposes an
    /// edit against a snapshot of the working copy (re-seeded from the live patch when idle),
    /// `Apply` swaps the pending proposal in and pushes the result onto the canvas, `Reject`
    /// discards it.
    #[cfg(feature = "assistant")]
    fn handle_assistant_action(&mut self, action: AssistantAction) {
        match action {
            AssistantAction::Submit(instruction) => {
                // Ignore a new request while one is already running — the panel's send button
                // stays live, but one worker at a time keeps the flow legible.
                if self.assistant.in_flight() {
                    return;
                }
                // Re-seed from the live patch when starting fresh (no proposal pending), so a
                // new request reflects any direct canvas edits since the last turn. Mid-
                // proposal, keep the working copy so a follow-up refines the same edit.
                if self.assistant.session.as_ref().map(EditSession::state)
                    != Some(EditState::Proposed)
                {
                    self.assistant.session = Some(EditSession::new(self.patch.to_doc()));
                }
                let Some(provider) = self.assistant.provider.clone() else {
                    self.assistant.status =
                        Some("no AI backend configured (assistant-net only)".to_owned());
                    return;
                };
                // Enforce the privacy policy before anything leaves the machine: offline mode
                // and missing consent both block the send (000110). A provider is already
                // configured here, so the key precondition is met — only the policy remains.
                if let Some(reason) = self.assistant.settings.privacy().blocked_reason() {
                    self.assistant.status = Some(format!("cannot send: {reason}"));
                    return;
                }
                // Snapshot everything the blocking call needs and run it off the UI thread.
                let doc = self
                    .assistant
                    .session
                    .as_ref()
                    .expect("seeded above")
                    .doc()
                    .clone();
                let registry = self.assistant.registry.clone();
                let config = self.assistant.settings.config().clone();
                let max_attempts = self.assistant.max_attempts;
                let (tx, rx) = mpsc::channel();
                let worker = std::thread::spawn(move || {
                    let out = vb_ai::edit_patch(
                        provider.as_ref(),
                        &registry,
                        &config,
                        &doc,
                        &instruction,
                        max_attempts,
                    );
                    let _ = tx.send((instruction, out));
                });
                self.assistant.rx = Some(rx);
                self.assistant.worker = Some(worker);
                self.assistant.status = Some("thinking…".to_owned());
            }
            AssistantAction::Apply => {
                let applied = self
                    .assistant
                    .session
                    .as_mut()
                    .and_then(|s| s.apply().cloned());
                if let Some(doc) = applied {
                    self.load_doc(&doc);
                    self.assistant.status = Some("applied the proposed edit".to_owned());
                }
            }
            AssistantAction::Reject => {
                if let Some(session) = self.assistant.session.as_mut() {
                    session.reject();
                    self.assistant.status = Some("rejected the proposed edit".to_owned());
                }
            }
        }
    }

    /// The active colour theme.
    #[must_use]
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Switches to the `kind` palette, queuing the egui base visuals for reinstall on
    /// the next frame. A no-op if that palette is already active.
    pub fn set_theme(&mut self, kind: ThemeKind) {
        if self.theme.kind != kind {
            self.theme = Theme::for_kind(kind);
            self.visuals_dirty = true;
        }
    }

    /// Serialises the **whole active patch** to a RON string (the `.vbp` project format).
    ///
    /// Filesystem-free — the caller decides where the text goes (a text box, the clipboard,
    /// a browser download) — so it is the serialization primitive the WASM build uses. Only
    /// structure is captured (nodes + edges); canvas-only state is intentionally dropped,
    /// matching [`PatchView::to_doc`].
    ///
    /// # Errors
    /// [`vb_project::ProjectError`] if the document cannot be serialised.
    pub fn export_project_ron(&self) -> Result<String, vb_project::ProjectError> {
        vb_project::to_ron(&self.patch.to_doc())
    }

    /// Replaces the active patch with the project parsed from `text` (a `.vbp` RON string).
    ///
    /// Parses and migrates via [`vb_project::from_ron`], then applies the document with
    /// [`PatchView::apply_doc`], which validates it and rebuilds the canvas as one undoable
    /// step. Also filesystem-free, so it is the import path for the browser.
    ///
    /// # Errors
    /// The parse/migration error from [`vb_project::from_ron`], or a message if the parsed
    /// document fails graph validation on apply.
    pub fn import_project_ron(&mut self, text: &str) -> Result<(), String> {
        let doc = vb_project::from_ron(text).map_err(|e| e.to_string())?;
        if self.patch.apply_doc(&doc, &self.registry) {
            Ok(())
        } else {
            Err("the project is structurally invalid and was not loaded".to_owned())
        }
    }

    /// Replaces the edited patch with `doc` — e.g. seeding the editor with the patch a
    /// plugin host is already playing — leaving **no** undo step back to the previous
    /// contents (so a freshly-loaded editor can't be undone into its demo patch).
    ///
    /// Returns `false` if `doc` fails validation, in which case the patch is unchanged.
    pub fn load_doc(&mut self, doc: &vb_project::ProjectDoc) -> bool {
        let ok = self.patch.apply_doc(doc, &self.registry);
        self.patch.clear_history();
        ok
    }

    /// Exports macro `id`'s inner patch to `dir/<name>.vbmod.ron`, returning the path.
    ///
    /// # Errors
    /// [`ModuleError::Build`] if `id` is not a macro (has no subpatch), or a serialize /
    /// filesystem error from [`MacroModule::write_to_dir`].
    pub fn export_macro_to_dir(
        &self,
        id: vb_core::NodeId,
        dir: &Path,
    ) -> Result<PathBuf, ModuleError> {
        let sub = self
            .patch
            .subpatch(id)
            .ok_or_else(|| ModuleError::Build(format!("node {} is not a macro", id.0)))?;
        sub.to_module(format!("macro-{}", id.0)).write_to_dir(dir)
    }

    /// Imports the module file at `path` as a new macro node at the current spawn point,
    /// selects it, and returns its id.
    ///
    /// # Errors
    /// Any [`ModuleError`] from reading, parsing, or rebuilding the module.
    pub fn import_macro_from_file(&mut self, path: &Path) -> Result<vb_core::NodeId, ModuleError> {
        let module = MacroModule::read_from_file(path)?;
        let sub = PatchView::from_module(&module)?;
        let half = Vec2::new(self.style.width * 0.5, self.style.title_height);
        let id = self
            .patch
            .add_macro_with_subpatch(self.spawn_at - half, sub);
        self.patch
            .click(self.spawn_at - half + Vec2::new(4.0, 4.0), &self.style);
        Ok(id)
    }

    /// Test/integration hook: a full wire drag between two viewport-local positions.
    pub fn drag_wire(&mut self, from_screen: Pos2, to_screen: Pos2) {
        let from = self.camera.screen_to_canvas(from_screen);
        let to = self.camera.screen_to_canvas(to_screen);
        self.patch.begin_wire(from, &self.style);
        self.patch.end_wire(to, &self.style);
    }
}

impl EditorApp {
    /// Draws the whole editor (toolbar, palette, inspector, canvas) into `ctx`.
    ///
    /// This is the frame-independent render entry: `impl eframe::App::update` calls it for
    /// the standalone/browser windows, and the plugin's `nih_plug_egui` host calls it
    /// directly inside its own egui context — one editor, drawn the same everywhere, no
    /// `eframe::Frame` required.
    pub fn render(&mut self, ctx: &egui::Context) {
        // Consume any assistant proposal a worker finished since the last frame, and keep the
        // UI ticking while one is still running so the poll actually fires.
        #[cfg(feature = "assistant")]
        {
            self.poll_assistant();
            if self.assistant.in_flight() {
                ctx.request_repaint();
            }
        }
        // Install the palette's egui base once, and again after any theme toggle, so
        // panels/buttons/combos match the canvas.
        if self.visuals_dirty {
            ctx.set_visuals(self.theme.egui_visuals());
            self.visuals_dirty = false;
        }
        self.toolbar_ui(ctx);
        match self.view_mode {
            // The node graph: palette + inspector side panels around the canvas.
            ViewMode::Structure => {
                egui::SidePanel::left("palette")
                    .resizable(true)
                    .show(ctx, |ui| self.palette_ui(ui));
                egui::SidePanel::right("inspector")
                    .resizable(true)
                    .show(ctx, |ui| self.inspector_ui(ui));
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| self.canvas_ui(ui));
            }
            // The control panel: one live-bound widget per node parameter, no wiring.
            ViewMode::Panel => {
                egui::CentralPanel::default().show(ctx, |ui| self.panel_ui(ui));
            }
        }
    }

    /// The top toolbar: the **Blocks / Panel** view switch first, then the macro breadcrumbs,
    /// view controls (zoom, snap, theme), and undo/redo. Shared by both view modes.
    fn toolbar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Switch between the node graph and the control panel.
                ui.selectable_value(&mut self.view_mode, ViewMode::Structure, "Blocks");
                ui.selectable_value(&mut self.view_mode, ViewMode::Panel, "Panel");
                ui.separator();
                ui.label("VST_bomb");
                ui.separator();
                // Breadcrumbs: root / macro#n / … — click any level to jump back.
                let depth = self.stack.len();
                if ui
                    .add_enabled(depth > 0, egui::Button::new("root").frame(false))
                    .clicked()
                {
                    while self.leave_macro() {}
                }
                for level in 0..depth {
                    ui.label("/");
                    let id = self.stack[level].1;
                    let last = level + 1 == depth;
                    if ui
                        .add_enabled(
                            !last,
                            egui::Button::new(format!("macro#{}", id.0)).frame(false),
                        )
                        .clicked()
                    {
                        while self.stack.len() > level + 1 {
                            self.leave_macro();
                        }
                    }
                }
                ui.separator();
                ui.label(format!("zoom {:.0}%", self.camera.zoom() * 100.0));
                if ui.button("reset view").clicked() {
                    self.camera = Camera::default();
                }
                ui.checkbox(&mut self.snap_to_grid, "snap");
                if ui
                    .button(format!("theme: {}", self.theme.kind.label()))
                    .clicked()
                {
                    self.set_theme(self.theme.kind.toggled());
                }
                ui.separator();
                if ui
                    .add_enabled(self.patch.can_undo(), egui::Button::new("undo"))
                    .clicked()
                {
                    self.patch.undo();
                }
                if ui
                    .add_enabled(self.patch.can_redo(), egui::Button::new("redo"))
                    .clicked()
                {
                    self.patch.redo();
                }
                if let Some(id) = self.patch.selected() {
                    ui.separator();
                    let kind = self.patch.graph().node(id).map_or("?", |n| n.kind.as_str());
                    ui.label(format!("selected: {kind}"));
                }
            });
        });
    }

    /// The current view surface (Blocks or Panel).
    #[must_use]
    pub fn view_mode(&self) -> ViewMode {
        self.view_mode
    }

    /// Switches the view surface (Blocks vs Panel).
    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
    }

    /// The control-panel bindings for the **active** patch: one [`PanelBinding`] per parameter
    /// of every node, in canvas draw order. This is the model the Panel surface renders and the
    /// unit tests assert against, independent of any egui frame.
    #[must_use]
    pub fn panel_widgets(&self) -> Vec<PanelBinding> {
        let mut out = Vec::new();
        for (id, node, _) in self.patch.draw_order() {
            for (index, slot) in self.patch.params(id).iter().enumerate() {
                out.push(PanelBinding {
                    node: id,
                    index,
                    label: format!("{} {}", node.kind, slot.param.name),
                    value: slot.value,
                    min: slot.param.min,
                    max: slot.param.max,
                });
            }
        }
        out
    }

    /// Renders the control panel: a scrollable list of labelled sliders, one per node
    /// parameter, each bound live to [`PatchView::set_param`] so tweaking a knob here edits the
    /// same value the inspector and the engine see.
    fn panel_ui(&mut self, ui: &mut Ui) {
        ui.heading("panel");
        let bindings = self.panel_widgets();
        if bindings.is_empty() {
            ui.weak("this patch has no parameters to control");
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for b in bindings {
                ui.horizontal(|ui| {
                    ui.add_sized([180.0, 18.0], egui::Label::new(&b.label).truncate());
                    let mut value = b.value;
                    let response = ui.add(egui::Slider::new(&mut value, b.min..=b.max));
                    if response.changed() {
                        self.patch.set_param(b.node, b.index, value);
                    }
                    if response.drag_stopped() || response.lost_focus() {
                        self.patch.end_gesture();
                    }
                });
            }
        });
    }
}

/// The standalone/browser launcher wiring: an [`EditorApp`] IS an eframe app, delegating
/// every frame to [`EditorApp::render`]. Behind the `app` feature so an embedder that owns
/// its own egui context (the plugin) does not pull eframe.
#[cfg(feature = "app")]
impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render(ctx);
    }
}

/// Opens the editor window and runs it to completion.
///
/// Native only — the browser entry point is [`crate::web::start`], which hands the same
/// [`EditorApp`] to eframe's `WebRunner` instead of opening an OS window.
///
/// # Errors
/// Returns [`eframe::Error`] if the native window/GL context cannot be created.
#[cfg(all(not(target_arch = "wasm32"), feature = "app"))]
pub fn run() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VST_bomb")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "VST_bomb",
        options,
        Box::new(|_cc| Ok(Box::new(EditorApp::new()))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_then_zoom_flows_through_to_the_camera() {
        let mut app = EditorApp::new();
        app.pan(Vec2::new(50.0, 20.0));
        let anchor = Pos2::new(200.0, 100.0);
        let pinned = app.camera().screen_to_canvas(anchor);
        app.zoom(anchor, 2.0);
        assert!((app.camera().zoom() - 2.0).abs() < f32::EPSILON);
        assert!((app.camera().canvas_to_screen(pinned) - anchor).length() < 0.5);
    }

    #[test]
    fn reset_state_is_the_default_camera() {
        let app = EditorApp::new();
        assert_eq!(*app.camera(), Camera::default());
    }

    #[test]
    fn the_editor_starts_dark_and_can_switch_themes() {
        let mut app = EditorApp::new();
        assert_eq!(app.theme().kind, ThemeKind::Dark, "dark by default");

        app.set_theme(ThemeKind::Light);
        assert_eq!(app.theme().kind, ThemeKind::Light);
        assert_eq!(app.theme(), Theme::light());

        // Toggling back returns the exact dark palette.
        app.set_theme(app.theme().kind.toggled());
        assert_eq!(app.theme(), Theme::dark());
    }

    #[test]
    fn param_unit_reads_the_name_suffix() {
        assert_eq!(param_unit("cutoff_hz"), "Hz");
        assert_eq!(param_unit("freq_hz"), "Hz");
        assert_eq!(param_unit("attack_s"), "s");
        assert_eq!(param_unit("glide_ms"), "ms");
        assert_eq!(param_unit("threshold_db"), "dB");
        assert_eq!(param_unit("detune_semitones"), "st");
        assert_eq!(param_unit("bpm"), "BPM");
        // Unitless parameters get no unit.
        assert_eq!(param_unit("resonance"), "");
        assert_eq!(param_unit("mix"), "");
        assert_eq!(param_unit("bias"), "");
    }

    #[test]
    fn the_view_starts_on_blocks_and_toggles_to_panel() {
        let mut app = EditorApp::new();
        assert_eq!(app.view_mode(), ViewMode::Structure);
        app.set_view_mode(ViewMode::Panel);
        assert_eq!(app.view_mode(), ViewMode::Panel);
        app.set_view_mode(ViewMode::Structure);
        assert_eq!(app.view_mode(), ViewMode::Structure);
    }

    #[test]
    fn the_panel_lists_every_node_parameter_of_the_patch() {
        let app = EditorApp::new();
        let bindings = app.panel_widgets();
        // The default Minitaur voice has many parameters; the panel exposes them all, one
        // widget per parameter, with the "<kind> <param>" labels.
        let expected: usize = app
            .patch()
            .draw_order()
            .map(|(id, _, _)| app.patch().params(id).len())
            .sum();
        assert_eq!(bindings.len(), expected);
        assert!(bindings.iter().any(|b| b.label == "ladder base_hz"));
        assert!(bindings.iter().any(|b| b.label == "vca bias"));
    }

    #[test]
    fn editing_a_panel_binding_updates_the_shared_parameter() {
        // A panel widget edits the same value the inspector/engine read (via set_param), so a
        // later panel_widgets() reflects the change.
        let mut app = EditorApp::new();
        let target = app
            .panel_widgets()
            .into_iter()
            .find(|b| b.label == "ladder resonance")
            .expect("ladder resonance is a panel control");
        let new_value = (target.value + 0.1).min(target.max);
        app.patch_mut()
            .set_param(target.node, target.index, new_value);
        let after = app
            .panel_widgets()
            .into_iter()
            .find(|b| b.node == target.node && b.index == target.index)
            .unwrap();
        assert!((after.value - new_value).abs() < 1e-4);
    }

    #[test]
    fn render_fills_the_context_at_any_size() {
        // The editor must lay out and run a full frame at whatever surface size it is given —
        // small or large — without panicking, and the standalone's `render` and the split
        // `render_chrome` + `canvas_ui` paths must both work.
        //
        // NOTE: this covers only our *layout* across sizes. It does NOT cover the plugin
        // window actually resizing: that lives in the host↔window surface layer (the host's
        // `request_resize`, baseview's framebuffer), which has no headless test surface — a
        // desync there (window grown but egui's surface not) cannot be reproduced in a
        // `cargo test`, only in a real host.
        let mut app = EditorApp::new();
        for size in [egui::vec2(640.0, 400.0), egui::vec2(1600.0, 1000.0)] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            // Both view surfaces must lay out and run a frame at either size.
            for mode in [ViewMode::Structure, ViewMode::Panel] {
                app.set_view_mode(mode);
                let ctx = egui::Context::default();
                let _ = ctx.run(input.clone(), |ctx| app.render(ctx));
            }
        }
    }

    #[test]
    fn the_demo_patch_is_on_the_canvas_and_wired() {
        let app = EditorApp::new();
        // The starting patch is the Minitaur voice (the plugin default): midi-in, glide,
        // 3 pitch-osc, 3 attenuvert levels, 2 add mixers, ladder, 2 adsr, vca, plus the LFO
        // cutoff-mod chain (lfo, cv-attenuvert, cv-add) = 17 nodes.
        assert_eq!(app.patch().graph().node_count(), 17);
        assert_eq!(app.patch().graph().edges().len(), 20);
        assert_eq!(app.patch().selected(), None);
    }

    #[test]
    fn spawn_node_adds_a_registry_node_and_selects_it() {
        let mut app = EditorApp::new();
        let before = app.patch().graph().node_count();

        let id = app.spawn_node("adsr").expect("adsr is a builtin");
        assert_eq!(app.patch().graph().node_count(), before + 1);
        assert_eq!(app.patch().selected(), Some(id), "spawned node is selected");
        let node = app.patch().graph().node(id).unwrap();
        assert_eq!(node.kind, "adsr");
        assert!(
            !node.inputs.is_empty() || !node.outputs.is_empty(),
            "registry nodes come with their real ports"
        );

        // The spawned frame is centred on the spawn point.
        let pos = app.patch().position(id).unwrap();
        let style = NodeStyle::default();
        assert!((pos.x + style.width * 0.5 - app.spawn_at.x).abs() < 1.0);

        assert_eq!(app.spawn_node("no-such-kind"), None);
        assert_eq!(app.patch().graph().node_count(), before + 1);
    }

    #[test]
    fn spawned_nodes_carry_registry_params_and_edits_clamp() {
        let mut app = EditorApp::new();
        let id = app.spawn_node("biquad").expect("builtin");

        let slots = app.patch().params(id);
        assert_eq!(slots.len(), 3, "kind, cutoff_hz, q");
        assert_eq!(slots[1].param.name, "cutoff_hz");
        assert!((slots[1].value - slots[1].param.default).abs() < f32::EPSILON);

        // The inspector writes through set_param, which clamps into range.
        assert_eq!(app.patch_mut().set_param(id, 1, 99_999.0), Some(20_000.0));
        assert!((app.patch().params(id)[1].value - 20_000.0).abs() < f32::EPSILON);
    }

    #[test]
    fn project_ron_round_trips_the_whole_patch() {
        let mut app = EditorApp::new();
        app.spawn_node("gain").expect("builtin");
        let expected_nodes = app.patch().graph().node_count();
        let expected_edges = app.patch().graph().edges().len();

        // Export the live patch to RON, then load it into a fresh editor.
        let ron = app.export_project_ron().expect("serialize");
        assert!(ron.contains("gain"), "the RON names the patch's kinds");

        let mut loaded = EditorApp::new();
        loaded.import_project_ron(&ron).expect("load valid project");
        assert_eq!(loaded.patch().graph().node_count(), expected_nodes);
        assert_eq!(loaded.patch().graph().edges().len(), expected_edges);
        // apply_doc lands as one undoable step.
        assert!(loaded.patch().can_undo(), "import is undoable");
    }

    #[test]
    fn load_doc_seeds_the_patch_with_no_undo_history() {
        // A doc from one editor (its demo + an extra node) loaded into another.
        let mut source = EditorApp::new();
        source.spawn_node("gain").expect("builtin");
        let doc = source.patch().to_doc();
        let nodes = doc.nodes.len();
        let edges = doc.edges.len();

        let mut app = EditorApp::new();
        assert!(app.load_doc(&doc), "a valid doc loads");
        assert_eq!(app.patch().graph().node_count(), nodes);
        assert_eq!(app.patch().graph().edges().len(), edges);
        // Seeding is not a user edit: there is nothing to undo back into.
        assert!(
            !app.patch().can_undo(),
            "a freshly loaded patch carries no undo history"
        );
    }

    #[test]
    fn importing_garbage_ron_is_a_clean_error_and_leaves_the_patch_intact() {
        let mut app = EditorApp::new();
        let before = app.patch().graph().node_count();
        let err = app.import_project_ron("this is not ron").unwrap_err();
        assert!(!err.is_empty(), "the failure is reported");
        assert_eq!(
            app.patch().graph().node_count(),
            before,
            "a failed import must not mutate the canvas"
        );
    }

    #[test]
    fn undo_cannot_walk_back_into_an_empty_canvas_but_undoes_spawns() {
        let mut app = EditorApp::new();
        assert!(!app.patch().can_undo(), "demo seeding left no history");

        let before = app.patch().graph().node_count();
        app.spawn_node("mix").expect("builtin");
        assert_eq!(app.patch().graph().node_count(), before + 1);

        assert!(app.patch_mut().undo());
        assert_eq!(app.patch().graph().node_count(), before);
        assert!(!app.patch().can_undo(), "only the spawn was undoable");
        assert!(app.patch_mut().redo());
        assert_eq!(app.patch().graph().node_count(), before + 1);
    }

    #[test]
    fn a_macro_exports_and_reimports_as_a_reusable_module() {
        let dir = std::env::temp_dir().join(format!("vbmod-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut app = EditorApp::new();
        // Build a macro with a real node inside it.
        let mac = app.patch_mut().add_macro(Pos2::new(500.0, 200.0));
        app.enter_macro(mac);
        app.spawn_node("gain").expect("gain inside the macro");
        app.leave_macro();
        let inner_before = app.patch().subpatch(mac).unwrap().graph().node_count();

        // Export it, then import the file back as a new macro node.
        let path = app.export_macro_to_dir(mac, &dir).expect("export");
        assert!(path.exists());
        let before = app.patch().graph().node_count();
        let imported = app.import_macro_from_file(&path).expect("import");

        assert_eq!(
            app.patch().graph().node_count(),
            before + 1,
            "one new macro"
        );
        assert!(app.patch().is_macro(imported), "the import is a macro");
        assert_eq!(app.patch().selected(), Some(imported), "and is selected");
        assert_eq!(
            app.patch().subpatch(imported).unwrap().graph().node_count(),
            inner_before,
            "the inner patch came across intact"
        );

        // A non-macro node cannot be exported.
        let node = app.spawn_node("gain").unwrap();
        assert!(app.export_macro_to_dir(node, &dir).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn macros_can_be_entered_edited_and_left() {
        let mut app = EditorApp::new();
        let mac = app.patch_mut().add_macro(Pos2::new(500.0, 200.0));
        assert!(app.patch().is_macro(mac));
        assert_eq!(app.depth(), 0);

        assert!(app.enter_macro(mac), "double-click path");
        assert_eq!(app.depth(), 1);
        // Inside: the boundary nodes are on the canvas; edits work as usual.
        assert_eq!(app.patch().graph().node_count(), 2, "macro-in + macro-out");
        let spawned = app.spawn_node("gain").expect("editing inside works");
        assert!(app.patch().graph().node(spawned).is_some());

        // A non-macro node cannot be entered.
        assert!(!app.enter_macro(spawned));

        assert!(app.leave_macro());
        assert_eq!(app.depth(), 0);
        // The edit survived the round trip.
        let sub = app.patch().subpatch(mac).expect("subpatch back in place");
        assert_eq!(sub.graph().node_count(), 3);
        assert!(!app.leave_macro(), "already at root");
    }

    #[test]
    fn the_demo_patch_params_come_from_the_registry() {
        let app = EditorApp::new();
        // The Minitaur voice's oscillators are pitch-tracking (`pitch-osc`).
        let osc = app
            .patch()
            .draw_order()
            .find(|(_, n, _)| n.kind == "pitch-osc")
            .map(|(id, _, _)| id)
            .unwrap();
        let names: Vec<_> = app
            .patch()
            .params(osc)
            .iter()
            .map(|s| s.param.name.clone())
            .collect();
        assert_eq!(names, vec!["waveform", "base_hz", "detune_semitones"]);
    }

    #[test]
    fn dragging_a_wire_reports_drop_target_validity_through_the_camera() {
        let mut app = EditorApp::new();
        // Use the small known-shape wiring patch (osc → filter → gain), not the shipped
        // Minitaur voice, so this test targets stable node kinds/positions.
        app.patch = sample_wiring_patch(&app.registry);
        app.pan(Vec2::new(-15.0, 30.0));
        app.zoom(Pos2::new(80.0, 40.0), 1.4);
        let s = NodeStyle::default();

        let gain = app
            .patch()
            .draw_order()
            .find(|(_, n, _)| n.kind == "gain")
            .map(|(id, _, _)| id)
            .unwrap();
        let osc = app
            .patch()
            .draw_order()
            .find(|(_, n, _)| n.kind == "oscillator")
            .map(|(id, _, _)| id)
            .unwrap();

        // Not dragging: no verdict anywhere.
        let cam = *app.camera();
        let gain_in = app
            .patch()
            .port_anchor(gain, PortDirection::Input, PortId(0), &s)
            .unwrap();
        assert_eq!(app.drop_target_ok(cam.canvas_to_screen(gain_in)), None);

        // Drag from the oscillator output: gain.in is occupied → invalid; the filter's
        // free control input is a signal mismatch → invalid; empty canvas → None.
        let osc_out = app
            .patch()
            .port_anchor(osc, PortDirection::Output, PortId(0), &s)
            .unwrap();
        app.patch_mut()
            .begin_wire(osc_out, &s)
            .expect("drag armed on the output port");
        assert_eq!(
            app.drop_target_ok(cam.canvas_to_screen(gain_in)),
            Some(false),
            "occupied input"
        );
        assert_eq!(
            app.drop_target_ok(cam.canvas_to_screen(Pos2::new(-3_000.0, -3_000.0))),
            None
        );
    }

    #[test]
    fn a_wire_can_be_dragged_between_ports_through_the_camera() {
        let mut app = EditorApp::new();
        // Small known-shape wiring patch (has a `gain` with a free output to drag from).
        app.patch = sample_wiring_patch(&app.registry);
        app.pan(Vec2::new(25.0, 40.0));
        app.zoom(Pos2::new(10.0, 10.0), 1.3);

        // The demo gain node's output is free; wire it back is impossible (cycle), so
        // add a fresh sink node and wire gain.out → sink.in.
        // Clear of the demo nodes (gain now sits at x≈540) so the drag can't grab a
        // neighbouring frame.
        let sink = app.patch_mut().add_node(
            GraphNode::new("sink", vec![Port::input("in", SignalType::Audio)], vec![]),
            Pos2::new(780.0, 120.0),
        );
        let edges_before = app.patch().graph().edges().len();

        let gain = app
            .patch()
            .draw_order()
            .find(|(_, n, _)| n.kind == "gain")
            .map(|(id, _, _)| id)
            .unwrap();
        let from = app
            .patch()
            .port_anchor(
                gain,
                PortDirection::Output,
                PortId(0),
                &NodeStyle::default(),
            )
            .unwrap();
        let to = app
            .patch()
            .port_anchor(sink, PortDirection::Input, PortId(0), &NodeStyle::default())
            .unwrap();

        let cam = *app.camera();
        app.drag_wire(cam.canvas_to_screen(from), cam.canvas_to_screen(to));

        let edges = app.patch().graph().edges();
        assert_eq!(edges.len(), edges_before + 1);
        let new = edges.last().unwrap();
        assert_eq!((new.from_node, new.to_node), (gain, sink));
    }

    /// Builds a saved macro module named `name` (a gain inside a macro) under `dir`.
    fn save_named_module(dir: &Path, name: &str) {
        let mut app = EditorApp::new();
        let mac = app.patch_mut().add_macro(Pos2::new(500.0, 200.0));
        app.enter_macro(mac);
        app.spawn_node("gain").expect("gain inside the macro");
        app.leave_macro();
        let module = app.patch().subpatch(mac).unwrap().to_module(name);
        module.write_to_dir(dir).expect("write module");
    }

    #[test]
    fn refresh_library_loads_modules_from_the_dir() {
        let dir = std::env::temp_dir().join(format!("vblib-app-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        save_named_module(&dir, "Warm Bass");
        save_named_module(&dir, "Bright Lead");

        let mut app = EditorApp::new();
        app.modules_dir = dir.clone();
        // Before a load the library is empty; refresh pulls both modules in.
        assert!(app.library.visible().is_empty());
        app.refresh_library();
        assert_eq!(app.library.visible().len(), 2, "both saved modules appear");

        // A live filter survives the (re)load: set_items keeps the query/tags.
        app.library.set_query("warm");
        app.refresh_library();
        let visible: Vec<_> = app
            .library
            .visible()
            .iter()
            .map(|i| i.name.clone())
            .collect();
        assert_eq!(
            visible,
            vec!["Warm Bass"],
            "the query still applies after reload"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_module_by_name_resolves_a_sanitized_stem_and_rejects_unknowns() {
        let dir = std::env::temp_dir().join(format!("vblib-app-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // The name has an unsafe char, so the file stem ("Acid-Bass") differs from the name;
        // resolution must go by the module's stored name, not the file stem.
        save_named_module(&dir, "Acid/Bass");

        let mut app = EditorApp::new();
        app.modules_dir = dir.clone();
        let before = app.patch().graph().node_count();

        let id = app
            .import_module_by_name("Acid/Bass")
            .expect("the module resolves by its stored name");
        assert_eq!(
            app.patch().graph().node_count(),
            before + 1,
            "one new macro"
        );
        assert!(app.patch().is_macro(id), "the import is a macro");
        assert_eq!(app.patch().selected(), Some(id), "and is selected");

        // An unknown name is a clean error that leaves the canvas untouched.
        let err = app.import_module_by_name("No Such Module").unwrap_err();
        assert!(matches!(err, ModuleError::Io(_)));
        assert_eq!(app.patch().graph().node_count(), before + 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_editor_renders_with_the_library_section_without_panicking() {
        // Drive a full headless frame (toolbar + palette incl. the library section +
        // inspector + canvas). A smoke test: the frame must build without panicking.
        let dir = std::env::temp_dir().join(format!("vblib-app-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        save_named_module(&dir, "Warm Bass");

        let mut app = EditorApp::new();
        app.modules_dir = dir.clone();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        // The first render lazily loaded the library from disk.
        assert!(app.library_loaded, "the library loads on first render");
        assert_eq!(app.library.visible().len(), 1, "the saved module is listed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_factory_patch_replaces_the_canvas_and_rejects_unknowns() {
        let mut app = EditorApp::new();
        // The starting canvas is already the Minitaur (17 nodes). Load a different factory
        // instrument and the whole canvas is replaced.
        assert!(app.load_factory_patch("Behringer Edge"), "Edge loads");
        assert_eq!(
            app.patch().graph().node_count(),
            8,
            "the canvas is now the Edge percussion voice"
        );
        // A freshly loaded factory patch carries no undo history (like load_doc).
        assert!(!app.patch().can_undo());

        // An unknown name is a clean no-op.
        let before = app.patch().graph().node_count();
        assert!(!app.load_factory_patch("No Such Synth"));
        assert_eq!(app.patch().graph().node_count(), before, "canvas untouched");
    }

    #[test]
    fn the_editor_renders_with_the_factory_section_without_panicking() {
        let mut app = EditorApp::new();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
    }

    #[test]
    fn clicking_a_node_selects_it_through_the_camera() {
        let mut app = EditorApp::new();
        // Known-shape patch: the oscillator sits at canvas (0, 0) so the click math is stable.
        app.patch = sample_wiring_patch(&app.registry);
        // Pan and zoom first so the screen→canvas mapping is non-trivial.
        app.pan(Vec2::new(37.0, -12.0));
        app.zoom(Pos2::new(64.0, 64.0), 1.5);

        // The oscillator's frame starts at canvas (0, 0); aim inside its title.
        let target = app.camera().canvas_to_screen(Pos2::new(10.0, 10.0));
        app.click(target);
        let selected = app.patch().selected().expect("oscillator selected");
        let kind = app.patch().graph().node(selected).unwrap().kind.clone();
        assert_eq!(kind, "oscillator");

        // Far away is empty canvas: selection clears.
        app.click(app.camera().canvas_to_screen(Pos2::new(-2_000.0, -2_000.0)));
        assert_eq!(app.patch().selected(), None);
    }

    // --- Assistant app-wiring (backlog 000108, threaded "A" step) ------------------------

    #[cfg(feature = "assistant")]
    fn reply_with(ron: &str) -> String {
        format!("Here is the edited patch.\n\n```ron\n{ron}\n```\n")
    }

    /// A `Send + Sync` provider double that returns a fixed reply on every call — the shape a
    /// worker thread needs (the crate's `ScriptedProvider` uses `RefCell`, so it is not
    /// `Sync`). One valid reply converges the repair loop on the first attempt.
    #[cfg(feature = "assistant")]
    struct FixedProvider(String);

    #[cfg(feature = "assistant")]
    impl vb_ai::AiProvider for FixedProvider {
        fn kind(&self) -> vb_ai::ProviderKind {
            vb_ai::ProviderKind::Anthropic
        }
        fn complete(
            &self,
            _request: &vb_ai::ChatRequest,
        ) -> Result<vb_ai::ChatResponse, vb_ai::AiError> {
            Ok(vb_ai::ChatResponse::text(self.0.clone()))
        }
    }

    /// Blocks until the in-flight worker delivers its proposal, then consumes it — the worker
    /// sends its single message before exiting, so joining makes the poll deterministic (no
    /// sleeps, no spin).
    #[cfg(feature = "assistant")]
    fn await_assistant(app: &mut EditorApp) {
        let worker = app.assistant.worker.take().expect("a request is in flight");
        worker.join().expect("assistant worker");
        assert!(
            app.poll_assistant(),
            "a joined worker's proposal must be consumed"
        );
    }

    /// Submitting spawns a worker; once it lands the session is Proposed and applying replaces
    /// the canvas with the provider's document (the example patch, distinct from the demo).
    #[cfg(feature = "assistant")]
    #[test]
    fn assistant_submit_then_apply_updates_the_canvas() {
        let mut app = EditorApp::new();
        app.assistant.settings.set_consent(true); // privacy gate: allow the send
        let before = app.patch().to_doc().nodes.len();
        let expected = vb_project::from_ron(vb_ai::EXAMPLE_PATCH_RON)
            .expect("example doc parses")
            .nodes
            .len();
        assert_ne!(
            before, expected,
            "the example must differ from the demo patch"
        );

        app.set_assistant_provider(std::sync::Arc::new(FixedProvider(reply_with(
            vb_ai::EXAMPLE_PATCH_RON,
        ))));

        app.handle_assistant_action(AssistantAction::Submit("rebuild it".to_owned()));
        assert!(app.assistant.in_flight(), "the request runs on a worker");
        await_assistant(&mut app);

        assert_eq!(
            app.assistant.session.as_ref().map(EditSession::state),
            Some(EditState::Proposed),
            "a valid reply must produce a proposal"
        );
        // Nothing is on the canvas yet — a proposal is a preview, not an edit.
        assert_eq!(app.patch().to_doc().nodes.len(), before);

        app.handle_assistant_action(AssistantAction::Apply);
        assert_eq!(
            app.patch().to_doc().nodes.len(),
            expected,
            "apply must push the proposed document onto the canvas"
        );
    }

    /// A worker's proposal is also consumed by the render loop's poll, not only the test hook.
    #[cfg(feature = "assistant")]
    #[test]
    fn a_worker_proposal_is_consumed_by_render() {
        let mut app = EditorApp::new();
        app.assistant.settings.set_consent(true); // privacy gate: allow the send
        app.set_assistant_provider(std::sync::Arc::new(FixedProvider(reply_with(
            vb_ai::EXAMPLE_PATCH_RON,
        ))));

        app.handle_assistant_action(AssistantAction::Submit("build it".to_owned()));
        // Join the worker so its message is queued, then let render() do the polling.
        app.assistant
            .worker
            .take()
            .expect("in flight")
            .join()
            .expect("worker");
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));

        assert!(!app.assistant.in_flight(), "render consumed the proposal");
        assert_eq!(
            app.assistant.session.as_ref().map(EditSession::state),
            Some(EditState::Proposed)
        );
    }

    /// Rejecting a proposal leaves the canvas untouched and clears the pending edit.
    #[cfg(feature = "assistant")]
    #[test]
    fn assistant_reject_leaves_the_canvas_untouched() {
        let mut app = EditorApp::new();
        app.assistant.settings.set_consent(true); // privacy gate: allow the send
        let before = app.patch().to_doc().nodes.len();
        app.set_assistant_provider(std::sync::Arc::new(FixedProvider(reply_with(
            vb_ai::EXAMPLE_PATCH_RON,
        ))));

        app.handle_assistant_action(AssistantAction::Submit("change it".to_owned()));
        await_assistant(&mut app);
        app.handle_assistant_action(AssistantAction::Reject);

        assert_eq!(
            app.assistant.session.as_ref().map(EditSession::state),
            Some(EditState::Idle)
        );
        assert_eq!(app.patch().to_doc().nodes.len(), before);
    }

    /// With no provider injected, Submit reports that no backend is configured, spawns no
    /// worker, and never mutates the canvas.
    #[cfg(feature = "assistant")]
    #[test]
    fn assistant_submit_without_a_provider_reports_no_backend() {
        let mut app = EditorApp::new();
        let before = app.patch().to_doc().nodes.len();

        app.handle_assistant_action(AssistantAction::Submit("do a thing".to_owned()));

        assert!(!app.assistant.in_flight(), "no worker without a provider");
        assert_eq!(app.patch().to_doc().nodes.len(), before);
        let status = app.assistant.status.as_deref().unwrap_or_default();
        assert!(
            status.contains("no AI backend"),
            "unexpected status: {status:?}"
        );
    }

    /// Even with a provider configured, the privacy policy blocks a send: no consent, and
    /// offline mode, each refuse the request without spawning a worker (000110).
    #[cfg(feature = "assistant")]
    #[test]
    fn the_privacy_policy_blocks_a_send_before_it_leaves() {
        let mut app = EditorApp::new();
        app.set_assistant_provider(std::sync::Arc::new(FixedProvider(reply_with(
            vb_ai::EXAMPLE_PATCH_RON,
        ))));

        // Consent is off by default → refused.
        app.handle_assistant_action(AssistantAction::Submit("go".to_owned()));
        assert!(!app.assistant.in_flight(), "no worker without consent");
        assert!(app
            .assistant
            .status
            .as_deref()
            .unwrap_or_default()
            .contains("cannot send"));

        // Consent given but offline → still refused.
        app.assistant.settings.set_consent(true);
        app.assistant.settings.set_offline(true);
        app.handle_assistant_action(AssistantAction::Submit("go".to_owned()));
        assert!(!app.assistant.in_flight(), "no worker while offline");

        // Consent + online → the send proceeds.
        app.assistant.settings.set_offline(false);
        app.handle_assistant_action(AssistantAction::Submit("go".to_owned()));
        assert!(app.assistant.in_flight(), "now the request runs");
        await_assistant(&mut app);
        assert_eq!(
            app.assistant.session.as_ref().map(EditSession::state),
            Some(EditState::Proposed)
        );
    }

    /// The editor renders a full frame — toolbar, palette (with the assistant section),
    /// inspector, canvas — without panicking under the `assistant` feature.
    #[cfg(feature = "assistant")]
    #[test]
    fn the_editor_renders_with_the_assistant_section_without_panicking() {
        let mut app = EditorApp::new();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
    }

    /// A `Send + Sync` provider that records the model of each request it sees — proof that the
    /// worker's request carries the config the Settings view-model holds.
    #[cfg(feature = "assistant")]
    struct RecordingProvider {
        reply: String,
        seen_model: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    }

    #[cfg(feature = "assistant")]
    impl vb_ai::AiProvider for RecordingProvider {
        fn kind(&self) -> vb_ai::ProviderKind {
            vb_ai::ProviderKind::Anthropic
        }
        fn complete(
            &self,
            request: &vb_ai::ChatRequest,
        ) -> Result<vb_ai::ChatResponse, vb_ai::AiError> {
            *self.seen_model.lock().unwrap() = Some(request.model.clone());
            Ok(vb_ai::ChatResponse::text(self.reply.clone()))
        }
    }

    /// The worker snapshots `settings.config()`, so a model chosen in the Settings view-model
    /// reaches the provider request.
    #[cfg(feature = "assistant")]
    #[test]
    fn the_worker_request_uses_the_settings_model() {
        let mut app = EditorApp::new();
        app.assistant.settings.set_consent(true); // privacy gate: allow the send
        app.assistant.settings.set_model("claude-custom-xyz");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        app.set_assistant_provider(std::sync::Arc::new(RecordingProvider {
            reply: reply_with(vb_ai::EXAMPLE_PATCH_RON),
            seen_model: seen.clone(),
        }));

        app.handle_assistant_action(AssistantAction::Submit("go".to_owned()));
        await_assistant(&mut app);

        assert_eq!(
            seen.lock().unwrap().as_deref(),
            Some("claude-custom-xyz"),
            "the request carries the model the Settings panel selected"
        );
    }

    /// The Settings key store round-trips through the host, and the readiness reflects it.
    #[cfg(feature = "assistant")]
    #[test]
    fn assistant_settings_key_and_readiness_round_trip_through_the_host() {
        let mut app = EditorApp::new();
        // Reach the host's own view-model and key store the panel would drive.
        assert!(!app
            .assistant
            .settings
            .key_present(app.assistant.keys.as_ref()));
        assert_eq!(
            app.assistant
                .settings
                .send_blocker(app.assistant.keys.as_ref()),
            Some(vb_ai::SendBlocker::NoConsent)
        );

        app.assistant.settings.set_consent(true);
        app.assistant
            .settings
            .store_key(app.assistant.keys.as_mut(), "sk-live")
            .expect("stores through the host key store");

        assert!(app.assistant.settings.is_ready(app.assistant.keys.as_ref()));
    }

    /// Without the `assistant-net` feature, applying the settings has no provider to connect,
    /// so it reports that rather than pretending to have connected.
    #[cfg(all(feature = "assistant", not(feature = "assistant-net")))]
    #[test]
    fn apply_settings_without_a_net_build_reports_no_connection() {
        let mut app = EditorApp::new();
        app.apply_assistant_settings();
        let status = app.assistant.status.as_deref().unwrap_or_default();
        assert!(
            status.contains("settings applied"),
            "unexpected status: {status:?}"
        );
    }
}
