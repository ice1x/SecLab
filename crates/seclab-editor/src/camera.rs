//! The canvas camera: an infinite 2D canvas seen through a pannable, zoomable viewport.
//!
//! All node positions live in *canvas* coordinates and never change when the user pans
//! or zooms; the camera alone maps them to *screen* coordinates. Zooming is anchored at
//! the pointer, so the canvas point under the cursor stays under the cursor.

use egui::{Pos2, Vec2};

/// Smallest allowed zoom (fits large patches on screen).
pub const MIN_ZOOM: f32 = 0.1;
/// Largest allowed zoom (close-up wire surgery).
pub const MAX_ZOOM: f32 = 8.0;

/// Maps between canvas space and screen space.
///
/// `screen = (canvas - offset) * zoom`, i.e. `offset` is the canvas point currently at
/// the screen origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Canvas point shown at the screen origin.
    pub offset: Pos2,
    /// Scale factor, clamped to [`MIN_ZOOM`]..=[`MAX_ZOOM`].
    zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            offset: Pos2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// Current zoom factor.
    #[must_use]
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Canvas → screen.
    #[must_use]
    pub fn canvas_to_screen(&self, p: Pos2) -> Pos2 {
        ((p - self.offset) * self.zoom).to_pos2()
    }

    /// Screen → canvas.
    #[must_use]
    pub fn screen_to_canvas(&self, p: Pos2) -> Pos2 {
        self.offset + p.to_vec2() / self.zoom
    }

    /// Pans by a drag delta given in *screen* pixels.
    pub fn pan_by(&mut self, screen_delta: Vec2) {
        self.offset -= screen_delta / self.zoom;
    }

    /// Multiplies zoom by `factor`, keeping the canvas point under `anchor` (a screen
    /// position, e.g. the pointer) fixed on screen. The result is clamped to
    /// [`MIN_ZOOM`]..=[`MAX_ZOOM`]; at the limits the anchor still holds.
    pub fn zoom_at(&mut self, anchor: Pos2, factor: f32) {
        let pinned = self.screen_to_canvas(anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        // Re-solve `screen = (canvas - offset) * zoom` for offset so `pinned` maps back
        // to `anchor` at the new zoom.
        self.offset = pinned - anchor.to_vec2() / self.zoom;
    }
}

/// Picks a grid step (in canvas units) whose on-screen spacing stays readable.
///
/// Steps are powers of two of the base spacing, so zooming halves/doubles the grid
/// instead of sliding it continuously.
#[must_use]
pub fn grid_step(zoom: f32) -> f32 {
    const BASE: f32 = 32.0;
    const MIN_SCREEN_SPACING: f32 = 12.0;
    let mut step = BASE;
    while step * zoom < MIN_SCREEN_SPACING {
        step *= 2.0;
    }
    while step * zoom >= MIN_SCREEN_SPACING * 2.0 && step > f32::EPSILON {
        step /= 2.0;
    }
    step
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(x: f32, y: f32) -> Pos2 {
        Pos2::new(x, y)
    }

    #[test]
    fn default_camera_is_identity() {
        let cam = Camera::default();
        assert_eq!(cam.canvas_to_screen(pos(10.0, -3.0)), pos(10.0, -3.0));
        assert_eq!(cam.screen_to_canvas(pos(7.5, 42.0)), pos(7.5, 42.0));
        assert!((cam.zoom() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn round_trip_is_identity() {
        let mut cam = Camera::default();
        cam.pan_by(Vec2::new(13.0, -8.0));
        cam.zoom_at(pos(100.0, 50.0), 1.7);
        let p = pos(-250.0, 419.0);
        let rt = cam.screen_to_canvas(cam.canvas_to_screen(p));
        assert!((rt - p).length() < 0.5, "{rt:?} != {p:?}");
    }

    #[test]
    fn pan_moves_content_with_the_drag() {
        let mut cam = Camera::default();
        let before = cam.canvas_to_screen(pos(0.0, 0.0));
        cam.pan_by(Vec2::new(30.0, -10.0));
        let after = cam.canvas_to_screen(pos(0.0, 0.0));
        // Dragging right/up moves the content right/up on screen.
        assert_eq!(after - before, Vec2::new(30.0, -10.0));
    }

    #[test]
    fn pan_respects_zoom() {
        let mut cam = Camera::default();
        cam.zoom_at(pos(0.0, 0.0), 2.0);
        let before = cam.canvas_to_screen(pos(5.0, 5.0));
        cam.pan_by(Vec2::new(40.0, 0.0));
        let after = cam.canvas_to_screen(pos(5.0, 5.0));
        // A 40-px drag moves content 40 px on screen regardless of zoom.
        assert_eq!(after - before, Vec2::new(40.0, 0.0));
    }

    #[test]
    fn zoom_keeps_the_anchor_fixed() {
        let mut cam = Camera::default();
        cam.pan_by(Vec2::new(-120.0, 35.0));
        let anchor = pos(300.0, 200.0);
        let pinned = cam.screen_to_canvas(anchor);
        cam.zoom_at(anchor, 2.5);
        assert!((cam.canvas_to_screen(pinned) - anchor).length() < 0.5);
        cam.zoom_at(anchor, 0.2);
        assert!((cam.canvas_to_screen(pinned) - anchor).length() < 0.5);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut cam = Camera::default();
        cam.zoom_at(Pos2::ZERO, 1_000.0);
        assert!((cam.zoom() - MAX_ZOOM).abs() < f32::EPSILON);
        cam.zoom_at(Pos2::ZERO, 1e-6);
        assert!((cam.zoom() - MIN_ZOOM).abs() < f32::EPSILON);
    }

    #[test]
    fn zoom_at_clamp_still_holds_the_anchor() {
        let mut cam = Camera::default();
        let anchor = pos(64.0, 64.0);
        let pinned = cam.screen_to_canvas(anchor);
        cam.zoom_at(anchor, 1_000.0); // clamps to MAX_ZOOM
        assert!((cam.canvas_to_screen(pinned) - anchor).length() < 0.5);
    }

    #[test]
    fn grid_step_stays_readable_across_zoom_range() {
        for zoom in [MIN_ZOOM, 0.25, 0.5, 1.0, 2.0, 4.0, MAX_ZOOM] {
            let screen_spacing = grid_step(zoom) * zoom;
            assert!(
                (12.0..24.0).contains(&screen_spacing),
                "zoom {zoom}: screen spacing {screen_spacing}"
            );
        }
    }
}
