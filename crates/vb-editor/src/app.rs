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
use vb_core::{GraphNode, Port, PortDirection, PortId, SignalType};

use vb_nodes::Registry;

use crate::camera::{grid_step, Camera};
use crate::layout::{layout_node, NodeLayout, NodeStyle};
use crate::palette::palette_entries;
use crate::patch::PatchView;

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
        }
    }
}

/// Starter content so the canvas isn't empty: the same oscillator → filter → gain
/// shape the standalone plays. Parameters come from the registry so the inspector has
/// something real to edit; the filter keeps an extra control input for wiring demos.
fn demo_patch(registry: &Registry) -> PatchView {
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
    let amp = patch.add_node_with_params(
        GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ),
        params_of("gain"),
        Pos2::new(360.0, 20.0),
    );
    patch
        .connect(osc, PortId(0), flt, PortId(0))
        .expect("demo osc→filter");
    patch
        .connect(flt, PortId(0), amp, PortId(0))
        .expect("demo filter→gain");
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

/// Port circle colour per signal type (audio = teal, control = amber, event = violet).
fn signal_color(signal: SignalType) -> Color32 {
    match signal {
        SignalType::Audio => Color32::from_rgb(0x2d, 0xb8, 0xa8),
        SignalType::Control => Color32::from_rgb(0xd9, 0xa4, 0x2b),
        SignalType::Event => Color32::from_rgb(0x9a, 0x6f, 0xe8),
    }
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
            self.moving = None;
            self.patch.end_gesture();
        }

        // Keyboard: delete/duplicate the selection — unless a text field (e.g. the
        // palette search box) owns the keyboard.
        if !ui.ctx().egui_wants_keyboard_input() {
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
        if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
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
                .map_or(Color32::from_gray(160), |p| signal_color(p.signal));
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
                    None => Color32::from_gray(200),
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

    /// Paints the adaptive background grid over `viewport`.
    fn paint_grid(&self, painter: &egui::Painter, viewport: Rect) {
        painter.rect_filled(viewport, 0.0, Color32::from_gray(24));

        let step = grid_step(self.camera.zoom());
        let stroke = Stroke::new(1.0, Color32::from_gray(40));
        let origin = viewport.min.to_vec2();

        // Canvas-space bounds of the visible viewport.
        let top_left = self.camera.screen_to_canvas(Pos2::ZERO);
        let bottom_right = self
            .camera
            .screen_to_canvas((viewport.max - viewport.min).to_pos2());

        let mut x = (top_left.x / step).floor() * step;
        while x <= bottom_right.x {
            let sx = self.camera.canvas_to_screen(Pos2::new(x, 0.0)).x + origin.x;
            painter.vline(sx, viewport.y_range(), stroke);
            x += step;
        }
        let mut y = (top_left.y / step).floor() * step;
        while y <= bottom_right.y {
            let sy = self.camera.canvas_to_screen(Pos2::new(0.0, y)).y + origin.y;
            painter.hline(viewport.x_range(), sy, stroke);
            y += step;
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

            let rounding = CornerRadius::same((4.0 * zoom).clamp(1.0, 6.0) as u8);
            painter.rect_filled(frame, rounding, Color32::from_gray(48));
            painter.rect_filled(title, rounding, Color32::from_gray(64));
            let outline = if selected {
                Stroke::new(2.0, Color32::from_rgb(0xff, 0xb0, 0x30))
            } else {
                Stroke::new(1.0, Color32::from_gray(90))
            };
            painter.rect_stroke(frame, rounding, outline, egui::StrokeKind::Outside);

            painter.text(
                title.center(),
                Align2::CENTER_CENTER,
                &node.kind,
                FontId::proportional((12.0 * zoom).clamp(8.0, 24.0)),
                Color32::from_gray(220),
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
                let mut fill = signal_color(port.signal);
                if target_ok == Some(false) && !is_grabbed {
                    fill = fill.gamma_multiply(0.3);
                }
                if target_ok == Some(true) {
                    painter.circle_stroke(
                        at,
                        port_r + (3.0 * zoom).clamp(2.0, 6.0),
                        Stroke::new(2.0, Color32::from_rgb(0x4c, 0xd9, 0x64)),
                    );
                }
                painter.circle_filled(at, port_r, fill);
                painter.circle_stroke(at, port_r, Stroke::new(1.0, Color32::from_gray(20)));
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
                        Color32::from_gray(170),
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
        // Copy the slots out so the sliders can borrow `ui` while edits go back
        // through set_param (which re-clamps).
        let slots: Vec<_> = self.patch.params(id).to_vec();
        for (i, slot) in slots.iter().enumerate() {
            let mut value = slot.value;
            let range = slot.param.min..=slot.param.max;
            let response = ui.add(egui::Slider::new(&mut value, range).text(&slot.param.name));
            if response.changed() {
                self.patch.set_param(id, i, value);
            }
            if response.drag_stopped() || response.lost_focus() {
                self.patch.end_gesture();
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

    /// Test/integration hook: a full wire drag between two viewport-local positions.
    pub fn drag_wire(&mut self, from_screen: Pos2, to_screen: Pos2) {
        let from = self.camera.screen_to_canvas(from_screen);
        let to = self.camera.screen_to_canvas(to_screen);
        self.patch.begin_wire(from, &self.style);
        self.patch.end_wire(to, &self.style);
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
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
        egui::Panel::left("palette")
            .resizable(true)
            .show(ui, |ui| self.palette_ui(ui));
        egui::Panel::right("inspector")
            .resizable(true)
            .show(ui, |ui| self.inspector_ui(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas_ui(ui));
    }
}

/// Opens the editor window and runs it to completion.
///
/// # Errors
/// Returns [`eframe::Error`] if the native window/GL context cannot be created.
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
    fn the_demo_patch_is_on_the_canvas_and_wired() {
        let app = EditorApp::new();
        assert_eq!(app.patch().graph().node_count(), 3);
        assert_eq!(app.patch().graph().edges().len(), 2, "osc→filter→gain");
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
        let osc = app
            .patch()
            .draw_order()
            .find(|(_, n, _)| n.kind == "oscillator")
            .map(|(id, _, _)| id)
            .unwrap();
        let names: Vec<_> = app
            .patch()
            .params(osc)
            .iter()
            .map(|s| s.param.name.clone())
            .collect();
        assert_eq!(names, vec!["waveform", "freq_hz"]);
    }

    #[test]
    fn dragging_a_wire_reports_drop_target_validity_through_the_camera() {
        let mut app = EditorApp::new();
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
        app.pan(Vec2::new(25.0, 40.0));
        app.zoom(Pos2::new(10.0, 10.0), 1.3);

        // The demo gain node's output is free; wire it back is impossible (cycle), so
        // add a fresh sink node and wire gain.out → sink.in.
        let sink = app.patch_mut().add_node(
            GraphNode::new("sink", vec![Port::input("in", SignalType::Audio)], vec![]),
            Pos2::new(560.0, 30.0),
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

    #[test]
    fn clicking_a_node_selects_it_through_the_camera() {
        let mut app = EditorApp::new();
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
}
