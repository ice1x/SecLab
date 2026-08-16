//! The editor window shell: an infinite canvas with pan, zoom, a grid, and nodes.
//!
//! Input mapping (Reaktor-style):
//! - drag on the canvas — pan
//! - scroll / pinch — zoom anchored at the pointer
//! - click — select the node under the cursor (raises it), click empty to deselect
//!
//! Wires, the palette, and node dragging land in the follow-up Epic 4 tasks; this
//! shell owns the [`Camera`] and the [`PatchView`] they all draw through.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use vb_core::{GraphNode, Port, SignalType};

use crate::camera::{grid_step, Camera};
use crate::layout::{layout_node, NodeLayout, NodeStyle};
use crate::patch::PatchView;

/// Root application state for the node editor window.
pub struct EditorApp {
    camera: Camera,
    patch: PatchView,
    style: NodeStyle,
}

impl Default for EditorApp {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            patch: demo_patch(),
            style: NodeStyle::default(),
        }
    }
}

/// Seed content until the palette (000043) lets the user add nodes: the same
/// oscillator → filter → gain shape the standalone plays.
fn demo_patch() -> PatchView {
    let mut patch = PatchView::new();
    patch.add_node(
        GraphNode::new(
            "oscillator",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ),
        Pos2::new(0.0, 0.0),
    );
    patch.add_node(
        GraphNode::new(
            "biquad",
            vec![
                Port::input("in", SignalType::Audio),
                Port::input("cutoff", SignalType::Control),
            ],
            vec![Port::output("out", SignalType::Audio)],
        ),
        Pos2::new(180.0, 10.0),
    );
    patch.add_node(
        GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ),
        Pos2::new(360.0, 20.0),
    );
    patch
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

        // Pan: drag anywhere on the canvas.
        if response.dragged() {
            self.camera.pan_by(response.drag_delta());
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

        self.paint_grid(&painter, viewport);
        self.paint_nodes(&painter, origin);
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
            for (anchor, port, align) in l
                .inputs
                .iter()
                .zip(&node.inputs)
                .map(|(a, p)| (a, p, Align2::LEFT_CENTER))
                .chain(
                    l.outputs
                        .iter()
                        .zip(&node.outputs)
                        .map(|(a, p)| (a, p, Align2::RIGHT_CENTER)),
                )
            {
                let at = to_screen(*anchor);
                painter.circle_filled(at, port_r, signal_color(port.signal));
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

    /// Test/integration hook: clicks at a viewport-local screen position.
    pub fn click(&mut self, screen: Pos2) {
        let canvas = self.camera.screen_to_canvas(screen);
        self.patch.click(canvas, &self.style);
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("VST_bomb");
                ui.separator();
                ui.label(format!("zoom {:.0}%", self.camera.zoom() * 100.0));
                if ui.button("reset view").clicked() {
                    self.camera = Camera::default();
                }
                if let Some(id) = self.patch.selected() {
                    ui.separator();
                    let kind = self.patch.graph().node(id).map_or("?", |n| n.kind.as_str());
                    ui.label(format!("selected: {kind}"));
                }
            });
        });
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
    fn the_demo_patch_is_on_the_canvas() {
        let app = EditorApp::new();
        assert_eq!(app.patch().graph().node_count(), 3);
        assert_eq!(app.patch().selected(), None);
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
