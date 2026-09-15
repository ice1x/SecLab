//! Control widgets for the inspector: the pure mapping between a pointer gesture and
//! a parameter's value, kept free of egui so it can be unit-tested.
//!
//! Every widget ultimately edits a [`vb_core::Parameter`] over its `[0, 1]` normalized
//! interval (the same one hosts automate). A **knob** turns vertical drag into a
//! normalized delta; a **switch** cycles a parameter's discrete positions; an
//! **XY-pad** maps a 2-D pointer position onto two parameters at once. The egui
//! rendering in `app.rs` is a thin shell over these functions.

use serde::{Deserialize, Serialize};
use vb_core::Parameter;

/// Which control the inspector draws for a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WidgetKind {
    /// A horizontal value slider (the classic control).
    Slider,
    /// A rotary knob driven by vertical drag. The default for continuous parameters.
    #[default]
    Knob,
    /// A stepped selector that cycles the parameter's discrete positions.
    Switch,
    /// A 2-D pad binding this parameter (X) to the next one (Y).
    XyPad,
    /// A vertical fader — the console/synth control, and the natural shape for an envelope
    /// segment (attack/decay/sustain/release read as a stack of faders).
    Fader,
}

impl WidgetKind {
    /// The widget kinds in a stable order, for building a picker.
    pub const ALL: [WidgetKind; 5] = [
        WidgetKind::Knob,
        WidgetKind::Fader,
        WidgetKind::Slider,
        WidgetKind::Switch,
        WidgetKind::XyPad,
    ];

    /// Short label for a picker / debugging.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            WidgetKind::Slider => "slider",
            WidgetKind::Knob => "knob",
            WidgetKind::Switch => "switch",
            WidgetKind::XyPad => "xy-pad",
            WidgetKind::Fader => "fader",
        }
    }

    /// The widget kind for a [`label`](Self::label), or `None` for an unrecognized string — the
    /// inverse used when restoring a saved panel layout from the project (which stores the label,
    /// not the enum). An unknown label falls back to the default control at the call site.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        WidgetKind::ALL.into_iter().find(|k| k.label() == label)
    }
}

/// Widget a freshly added parameter gets. Envelope segments default to a **fader** — an
/// attack/decay/sustain/release stack reads far more naturally as faders than as dials, which
/// is how hardware and most soft-synths draw them. Everything else defaults to a **knob**, a
/// predictable choice that never misreads a continuous range as an enum (only the registry truly
/// knows which parameters are discrete). Users retype any of it per parameter.
#[must_use]
pub fn default_widget(param: &Parameter) -> WidgetKind {
    if is_envelope_param(&param.name) {
        WidgetKind::Fader
    } else {
        WidgetKind::Knob
    }
}

/// Whether a parameter name reads as an envelope segment (a `*_s` time — attack/decay/release —
/// or the `sustain` level), so it should default to a fader.
#[must_use]
fn is_envelope_param(name: &str) -> bool {
    name.ends_with("_s") || name == "sustain"
}

/// Vertical drag, in screen pixels, that sweeps a knob across its whole range. Deliberately
/// generous: a full sweep should take a real gesture, so small moves give fine control (the
/// "no room to pick a value precisely" complaint was a too-short sweep).
pub const KNOB_RANGE_PX: f32 = 320.0;

/// Multiplier applied to a knob's drag distance while a fine-drag modifier (Shift) is held, so
/// the same pointer motion covers a quarter of the range — precise trimming on demand.
pub const KNOB_FINE_FACTOR: f32 = 0.25;

/// Whether a parameter reads on a **logarithmic** scale — a time (`_s`, `_ms`) or frequency
/// (`_hz`) control, where the musically useful detail lives at the low end. Drawn on a linear
/// slider, 0.01 s and 0.3 s are a few pixels apart while 3–5 s eat most of the travel; a log
/// scale spreads the low end out so those values are actually reachable. Levels (`sustain`,
/// `resonance`), gains (`_db`, already log-domain) and bipolar ranges (`detune_semitones`) stay
/// linear.
#[must_use]
pub fn param_is_log(name: &str) -> bool {
    name.ends_with("_hz") || name.ends_with("_ms") || name.ends_with("_s")
}

/// Smallest positive value a logarithmic slider spreads travel down to when its range starts at
/// zero (`egui::Slider::smallest_positive`). egui's default is `1e-6`, which for a `0..5 s`
/// envelope hands the bottom ~45% of the fader to sub-millisecond times nobody dials and crams the
/// useful 0.01–1 s into the top — so dragging down falls into a dead zone that snaps to `0.000`
/// (the "adsr decay won't leave zero" complaint). A 1 ms floor drops that dead zone: 0.001 s is
/// the practical minimum, and 0.01–1 s then occupies the lower-middle of the travel where it is
/// easy to grab.
pub const LOG_SMALLEST_POSITIVE: f64 = 0.001;

/// New normalized knob value after dragging `dy` screen pixels from `norm`.
///
/// Screen Y grows downward, so dragging **up** (negative `dy`) raises the value —
/// the direction people expect from a knob. The result stays in `[0, 1]`.
#[must_use]
pub fn knob_drag(norm: f32, dy: f32) -> f32 {
    (norm - dy / KNOB_RANGE_PX).clamp(0.0, 1.0)
}

/// Largest integer span a range may have and still be treated as a discrete enum by a
/// switch (a `0..=8` range = nine positions).
pub const SWITCH_MAX_SPAN: f32 = 8.0;

/// The discrete values a switch cycles for `param`.
///
/// A small integer-stepped range (integral bounds, `1..=SWITCH_MAX_SPAN` span) — how
/// the registry encodes enums like waveform or filter kind — yields one value per
/// integer. Anything else collapses to a two-state `[min, max]` toggle, so a switch is
/// always usable even on a continuous parameter.
#[must_use]
pub fn switch_values(param: &Parameter) -> Vec<f32> {
    let span = param.max - param.min;
    if span <= 0.0 {
        return vec![param.min];
    }
    let integral = param.min.fract() == 0.0 && param.max.fract() == 0.0;
    let steps = span.round();
    if integral && (span - steps).abs() < 1e-4 && (1.0..=SWITCH_MAX_SPAN).contains(&steps) {
        (0..=steps as u32).map(|i| param.min + i as f32).collect()
    } else {
        vec![param.min, param.max]
    }
}

/// Index of the switch position nearest `value`.
#[must_use]
pub fn switch_index(param: &Parameter, value: f32) -> usize {
    let values = switch_values(param);
    values
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (**a - value).abs().total_cmp(&(**b - value).abs()))
        .map_or(0, |(i, _)| i)
}

/// Value after stepping the switch one position `forward` (or back), wrapping around.
#[must_use]
pub fn switch_step(param: &Parameter, value: f32, forward: bool) -> f32 {
    let values = switch_values(param);
    let n = values.len();
    let i = switch_index(param, value);
    let next = if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    };
    values[next]
}

/// Maps a pad-local unit position (`0..1`, origin top-left) to `(x_norm, y_norm)`.
///
/// The Y axis is flipped so the **top** of the pad is each parameter's maximum, which
/// is how XY-pads conventionally read.
#[must_use]
pub fn xy_pad_to_norm(px: f32, py: f32) -> (f32, f32) {
    (px.clamp(0.0, 1.0), (1.0 - py).clamp(0.0, 1.0))
}

/// Inverse of [`xy_pad_to_norm`]: `(x_norm, y_norm)` back to a pad-local unit position.
#[must_use]
pub fn xy_norm_to_pad(x_norm: f32, y_norm: f32) -> (f32, f32) {
    (x_norm.clamp(0.0, 1.0), 1.0 - y_norm.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cont() -> Parameter {
        Parameter::new("gain", 0.0, 2.0, 1.0)
    }

    fn enum4() -> Parameter {
        // How the registry models a four-way enum (e.g. waveform).
        Parameter::new("waveform", 0.0, 3.0, 0.0)
    }

    fn fractional() -> Parameter {
        Parameter::new("q", 0.1, 10.0, 0.707)
    }

    #[test]
    fn label_round_trips_through_from_label() {
        for kind in WidgetKind::ALL {
            assert_eq!(WidgetKind::from_label(kind.label()), Some(kind));
        }
        assert_eq!(WidgetKind::from_label("nonsense"), None);
    }

    #[test]
    fn continuous_params_default_to_a_knob() {
        assert_eq!(default_widget(&cont()), WidgetKind::Knob);
        assert_eq!(default_widget(&enum4()), WidgetKind::Knob);
        assert_eq!(WidgetKind::default(), WidgetKind::Knob);
    }

    #[test]
    fn envelope_segments_default_to_a_fader() {
        for name in ["attack_s", "decay_s", "release_s", "sustain"] {
            let p = Parameter::new(name, 0.0, 1.0, 0.0);
            assert_eq!(
                default_widget(&p),
                WidgetKind::Fader,
                "{name} should be a fader"
            );
        }
        // A non-envelope parameter (even one in seconds-adjacent naming) stays a knob.
        assert_eq!(
            default_widget(&Parameter::new("glide_ms", 0.0, 5000.0, 50.0)),
            WidgetKind::Knob
        );
        assert_eq!(
            default_widget(&Parameter::new("freq_hz", 20.0, 20000.0, 440.0)),
            WidgetKind::Knob
        );
    }

    #[test]
    fn knob_drag_up_raises_and_down_lowers_and_clamps() {
        // Half a full sweep upward (negative dy) from the middle → near the top.
        let up = knob_drag(0.5, -KNOB_RANGE_PX * 0.5);
        assert!(
            (up - 1.0).abs() < 1e-6,
            "half a sweep up from mid, got {up}"
        );
        let down = knob_drag(0.5, KNOB_RANGE_PX * 0.5);
        assert!(down.abs() < 1e-6, "half a sweep down from mid, got {down}");
        // Overshoot is clamped to the unit interval.
        assert_eq!(knob_drag(0.9, -KNOB_RANGE_PX), 1.0);
        assert_eq!(knob_drag(0.1, KNOB_RANGE_PX), 0.0);
    }

    #[test]
    fn switch_values_split_enums_from_continuous_ranges() {
        assert_eq!(switch_values(&enum4()), vec![0.0, 1.0, 2.0, 3.0]);
        // A fractional range is not an enum: it collapses to a two-state toggle.
        assert_eq!(switch_values(&fractional()), vec![0.1, 10.0]);
        // A zero-width range yields a single fixed position.
        assert_eq!(
            switch_values(&Parameter::new("x", 1.0, 1.0, 1.0)),
            vec![1.0]
        );
        // Too many integer steps to be an enum → toggle only.
        let wide = Parameter::new("n", 0.0, 100.0, 0.0);
        assert_eq!(switch_values(&wide), vec![0.0, 100.0]);
    }

    #[test]
    fn switch_index_snaps_to_the_nearest_position() {
        let p = enum4();
        assert_eq!(switch_index(&p, 0.0), 0);
        assert_eq!(switch_index(&p, 1.6), 2, "rounds to the closer integer");
        assert_eq!(switch_index(&p, 2.9), 3);
    }

    #[test]
    fn switch_step_cycles_and_wraps_both_ways() {
        let p = enum4();
        assert_eq!(switch_step(&p, 0.0, true), 1.0);
        assert_eq!(switch_step(&p, 3.0, true), 0.0, "forward wraps at the top");
        assert_eq!(switch_step(&p, 0.0, false), 3.0, "back wraps at the bottom");
        // A two-state toggle flips between its ends.
        let q = fractional();
        assert_eq!(switch_step(&q, 0.1, true), 10.0);
        assert_eq!(switch_step(&q, 10.0, true), 0.1);
    }

    #[test]
    fn log_scale_covers_time_and_frequency_but_not_levels_or_bipolar() {
        // Time and frequency controls read logarithmically.
        for name in [
            "attack_s",
            "decay_s",
            "release_s",
            "glide_ms",
            "base_hz",
            "cutoff_hz",
            "amount_hz",
            "freq_hz",
        ] {
            assert!(param_is_log(name), "{name} should be logarithmic");
        }
        // Levels, unitless amounts, gains and bipolar ranges stay linear.
        for name in [
            "sustain",
            "resonance",
            "mix",
            "bias",
            "threshold_db",
            "detune_semitones",
            "bpm",
        ] {
            assert!(!param_is_log(name), "{name} should stay linear");
        }
    }

    #[test]
    fn xy_mapping_flips_y_and_round_trips() {
        // Top-left of the pad = min X, max Y.
        assert_eq!(xy_pad_to_norm(0.0, 0.0), (0.0, 1.0));
        // Bottom-right = max X, min Y.
        assert_eq!(xy_pad_to_norm(1.0, 1.0), (1.0, 0.0));
        // Round-trip through both directions.
        let (px, py) = (0.3, 0.8);
        let (xn, yn) = xy_pad_to_norm(px, py);
        let (rx, ry) = xy_norm_to_pad(xn, yn);
        assert!((rx - px).abs() < 1e-6 && (ry - py).abs() < 1e-6);
        // Out-of-pad positions are clamped.
        assert_eq!(xy_pad_to_norm(-0.5, 1.5), (0.0, 0.0));
    }
}
