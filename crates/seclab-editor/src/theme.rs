//! The editor's visual theme: one named palette of canvas, node, wire and text colours
//! plus a few metrics, so every painter call reads a semantic role instead of a literal
//! `Color32`. Two palettes ship — a deep dark studio look and a light one — and the
//! canvas, nodes and inspector widgets all recolour together when the theme changes.
//!
//! The struct is plain data: choosing a palette and mapping a [`SignalType`] to its
//! colour are pure and unit-tested; [`Theme::egui_visuals`] hands egui a matching base
//! so panels, buttons and combos sit in the same world as the canvas.

use egui::{Color32, Visuals};
use seclab_core::SignalType;

/// Which palette the editor paints with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    /// Deep neutral-dark studio palette (default).
    #[default]
    Dark,
    /// Light palette for bright rooms / documentation shots.
    Light,
}

impl ThemeKind {
    /// Short label for a toggle.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Dark => "dark",
            ThemeKind::Light => "light",
        }
    }

    /// The other palette — for a one-button toggle.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            ThemeKind::Dark => ThemeKind::Light,
            ThemeKind::Light => ThemeKind::Dark,
        }
    }
}

/// A resolved palette: every colour the canvas and widgets paint with, by role.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Which palette this is.
    pub kind: ThemeKind,
    /// Canvas background behind the grid.
    pub canvas_bg: Color32,
    /// Minor grid lines.
    pub grid_line: Color32,
    /// Major grid lines (every few cells) — a touch stronger than [`Self::grid_line`].
    pub grid_line_strong: Color32,
    /// Node body fill (bottom of the vertical gradient).
    pub node_fill: Color32,
    /// Node body fill at the top of the gradient — a hair lighter, for depth.
    pub node_fill_top: Color32,
    /// Node title bar.
    pub node_title: Color32,
    /// Node outline when unselected.
    pub node_outline: Color32,
    /// Node outline + glow when selected.
    pub node_outline_selected: Color32,
    /// Soft shadow cast under a node.
    pub node_shadow: Color32,
    /// Primary text.
    pub text: Color32,
    /// Secondary / dimmed text and labels.
    pub text_dim: Color32,
    /// Accent used by active widget fills and selection highlights.
    pub accent: Color32,
    /// Colour for an audio-signal port / wire.
    pub audio: Color32,
    /// Colour for a control-signal port / wire.
    pub control: Color32,
    /// Colour for an event-signal port / wire.
    pub event: Color32,
    /// Corner radius (canvas units) for node frames.
    pub node_rounding: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// The palette for `kind`.
    #[must_use]
    pub fn for_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Dark => Self::dark(),
            ThemeKind::Light => Self::light(),
        }
    }

    /// The deep-dark studio palette.
    #[must_use]
    pub fn dark() -> Self {
        Self {
            kind: ThemeKind::Dark,
            canvas_bg: Color32::from_rgb(0x15, 0x17, 0x1c),
            grid_line: Color32::from_rgb(0x24, 0x27, 0x2e),
            grid_line_strong: Color32::from_rgb(0x30, 0x34, 0x3d),
            node_fill: Color32::from_rgb(0x26, 0x2b, 0x33),
            node_fill_top: Color32::from_rgb(0x30, 0x36, 0x40),
            node_title: Color32::from_rgb(0x3a, 0x41, 0x4d),
            node_outline: Color32::from_rgb(0x51, 0x59, 0x66),
            node_outline_selected: Color32::from_rgb(0xff, 0xb0, 0x30),
            node_shadow: Color32::from_black_alpha(90),
            text: Color32::from_rgb(0xe6, 0xe9, 0xee),
            text_dim: Color32::from_rgb(0x9a, 0xa1, 0xac),
            accent: Color32::from_rgb(0x2d, 0xb8, 0xa8),
            audio: Color32::from_rgb(0x2d, 0xb8, 0xa8),
            control: Color32::from_rgb(0xd9, 0xa4, 0x2b),
            event: Color32::from_rgb(0x9a, 0x6f, 0xe8),
            node_rounding: 6.0,
        }
    }

    /// The light palette.
    #[must_use]
    pub fn light() -> Self {
        Self {
            kind: ThemeKind::Light,
            canvas_bg: Color32::from_rgb(0xec, 0xee, 0xf1),
            grid_line: Color32::from_rgb(0xd7, 0xdb, 0xe0),
            grid_line_strong: Color32::from_rgb(0xc2, 0xc8, 0xd0),
            node_fill: Color32::from_rgb(0xfb, 0xfc, 0xfd),
            node_fill_top: Color32::from_rgb(0xff, 0xff, 0xff),
            node_title: Color32::from_rgb(0xdd, 0xe2, 0xe8),
            node_outline: Color32::from_rgb(0xb4, 0xbc, 0xc6),
            node_outline_selected: Color32::from_rgb(0xe6, 0x8a, 0x00),
            node_shadow: Color32::from_black_alpha(36),
            text: Color32::from_rgb(0x1b, 0x1f, 0x24),
            text_dim: Color32::from_rgb(0x5c, 0x64, 0x6e),
            accent: Color32::from_rgb(0x12, 0x8a, 0x7d),
            audio: Color32::from_rgb(0x12, 0x8a, 0x7d),
            control: Color32::from_rgb(0xb5, 0x7d, 0x00),
            event: Color32::from_rgb(0x6b, 0x45, 0xc4),
            node_rounding: 6.0,
        }
    }

    /// Colour for a port or wire carrying `signal`.
    #[must_use]
    pub fn signal_color(&self, signal: SignalType) -> Color32 {
        match signal {
            SignalType::Audio => self.audio,
            SignalType::Control => self.control,
            SignalType::Event => self.event,
        }
    }

    /// An egui [`Visuals`] matching this palette, so panels/buttons/combos share the
    /// canvas world: correct dark/light base, panel fill, and the accent as the
    /// selection colour.
    #[must_use]
    pub fn egui_visuals(&self) -> Visuals {
        let mut v = match self.kind {
            ThemeKind::Dark => Visuals::dark(),
            ThemeKind::Light => Visuals::light(),
        };
        v.panel_fill = self.node_fill;
        v.override_text_color = Some(self.text);
        v.selection.bg_fill = self.accent.gamma_multiply(0.55);
        v.selection.stroke = egui::Stroke::new(1.0_f32, self.accent);
        v.hyperlink_color = self.accent;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(c: Color32) -> u32 {
        c.r() as u32 + c.g() as u32 + c.b() as u32
    }

    #[test]
    fn dark_and_light_are_distinct_and_correctly_ordered() {
        let (d, l) = (Theme::dark(), Theme::light());
        assert_eq!(d.kind, ThemeKind::Dark);
        assert_eq!(l.kind, ThemeKind::Light);
        // The dark canvas is darker than the light one; text inverts to stay readable.
        assert!(luma(d.canvas_bg) < luma(l.canvas_bg), "dark bg is darker");
        assert!(
            luma(d.text) > luma(d.canvas_bg),
            "dark text reads on dark bg"
        );
        assert!(
            luma(l.text) < luma(l.canvas_bg),
            "light text reads on light bg"
        );
        assert_ne!(d, l);
    }

    #[test]
    fn for_kind_and_default_pick_the_right_palette() {
        assert_eq!(Theme::for_kind(ThemeKind::Dark), Theme::dark());
        assert_eq!(Theme::for_kind(ThemeKind::Light), Theme::light());
        assert_eq!(Theme::default(), Theme::dark());
        assert_eq!(ThemeKind::default(), ThemeKind::Dark);
    }

    #[test]
    fn signal_colors_are_distinct_per_type() {
        let t = Theme::dark();
        let (a, c, e) = (
            t.signal_color(SignalType::Audio),
            t.signal_color(SignalType::Control),
            t.signal_color(SignalType::Event),
        );
        assert_eq!(a, t.audio);
        assert_eq!(c, t.control);
        assert_eq!(e, t.event);
        assert!(a != c && c != e && a != e, "each signal reads differently");
    }

    #[test]
    fn toggling_the_kind_round_trips() {
        assert_eq!(ThemeKind::Dark.toggled(), ThemeKind::Light);
        assert_eq!(ThemeKind::Light.toggled(), ThemeKind::Dark);
        assert_eq!(ThemeKind::Dark.toggled().toggled(), ThemeKind::Dark);
        assert_eq!(ThemeKind::Dark.label(), "dark");
    }

    #[test]
    fn egui_visuals_follow_the_palette() {
        assert!(
            Theme::dark().egui_visuals().dark_mode,
            "dark → dark visuals"
        );
        assert!(
            !Theme::light().egui_visuals().dark_mode,
            "light → light visuals"
        );
        assert_eq!(
            Theme::dark().egui_visuals().panel_fill,
            Theme::dark().node_fill
        );
    }
}
