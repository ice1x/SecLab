//! The editor window shell: an infinite canvas with pan, zoom, and an adaptive grid.
//!
//! Input mapping (Reaktor-style):
//! - drag with the primary button (or middle button) on empty canvas — pan
//! - scroll / pinch — zoom anchored at the pointer
//!
//! Node rendering, wires, and the palette land in the follow-up Epic 4 tasks; this
//! shell owns the [`Camera`] they will all draw through.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::camera::{grid_step, Camera};

/// Root application state for the node editor window.
#[derive(Default)]
pub struct EditorApp {
    camera: Camera,
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

    /// Draws the canvas into `ui` and applies pan/zoom input.
    ///
    /// Split out of [`eframe::App::ui`] so integration code (and later the
    /// standalone) can embed the canvas in any container.
    pub fn canvas_ui(&mut self, ui: &mut Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let viewport = response.rect;

        // Pan: drag anywhere on empty canvas.
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
                self.camera
                    .zoom_at(pointer - viewport.min.to_vec2(), factor);
            }
        }

        self.paint_grid(&painter, viewport);
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

        // Canvas origin marker so panning has a landmark until nodes land.
        let o = self.camera.canvas_to_screen(Pos2::ZERO) + origin;
        painter.circle_stroke(o, 4.0, Stroke::new(1.5, Color32::from_gray(120)));
    }

    /// Test/integration hook: pans the camera as a drag by `delta` screen pixels would.
    pub fn pan(&mut self, delta: Vec2) {
        self.camera.pan_by(delta);
    }

    /// Test/integration hook: zooms as a scroll at `anchor` (viewport-local) would.
    pub fn zoom(&mut self, anchor: Pos2, factor: f32) {
        self.camera.zoom_at(anchor, factor);
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
}
