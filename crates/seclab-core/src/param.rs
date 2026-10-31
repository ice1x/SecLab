//! Node parameters: named, ranged control values with host-automation normalization.

use serde::{Deserialize, Serialize};

/// A named parameter with an inclusive `[min, max]` range and a default value.
///
/// Hosts automate parameters over the unit interval `[0, 1]`; [`Parameter::normalize`]
/// and [`Parameter::denormalize`] convert between the plain value and that interval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// Human-readable label shown in the editor / host.
    pub name: String,
    /// Inclusive lower bound.
    pub min: f32,
    /// Inclusive upper bound.
    pub max: f32,
    /// Default value (already clamped into range by [`Parameter::new`]).
    pub default: f32,
}

impl Parameter {
    /// Creates a parameter, clamping `default` into `[min, max]`.
    #[must_use]
    pub fn new(name: impl Into<String>, min: f32, max: f32, default: f32) -> Self {
        Self {
            name: name.into(),
            min,
            max,
            default: default.clamp(min, max),
        }
    }

    /// Clamps `value` into `[min, max]`.
    #[must_use]
    pub fn clamp(&self, value: f32) -> f32 {
        value.clamp(self.min, self.max)
    }

    /// Maps a plain `value` to `[0, 1]`. A zero-width range maps to `0.0`.
    #[must_use]
    pub fn normalize(&self, value: f32) -> f32 {
        let span = self.max - self.min;
        if span <= 0.0 {
            0.0
        } else {
            ((value - self.min) / span).clamp(0.0, 1.0)
        }
    }

    /// Maps a normalized `t` in `[0, 1]` back to a plain value in `[min, max]`.
    #[must_use]
    pub fn denormalize(&self, t: f32) -> f32 {
        self.min + t.clamp(0.0, 1.0) * (self.max - self.min)
    }
}

#[cfg(test)]
mod tests {
    use super::Parameter;

    #[test]
    fn new_clamps_default_into_range() {
        assert_eq!(Parameter::new("p", 0.0, 10.0, 20.0).default, 10.0);
        assert_eq!(Parameter::new("p", 0.0, 10.0, -5.0).default, 0.0);
    }

    #[test]
    fn clamp_bounds_values() {
        let p = Parameter::new("cutoff", 20.0, 20_000.0, 1_000.0);
        assert_eq!(p.clamp(10.0), 20.0);
        assert_eq!(p.clamp(30_000.0), 20_000.0);
        assert_eq!(p.clamp(500.0), 500.0);
    }

    #[test]
    fn normalize_maps_range_to_unit_interval() {
        let p = Parameter::new("gain", 0.0, 10.0, 5.0);
        assert_eq!(p.normalize(0.0), 0.0);
        assert_eq!(p.normalize(5.0), 0.5);
        assert_eq!(p.normalize(10.0), 1.0);
        // Out-of-range inputs are clamped after mapping.
        assert_eq!(p.normalize(-1.0), 0.0);
        assert_eq!(p.normalize(11.0), 1.0);
    }

    #[test]
    fn denormalize_is_inverse_of_normalize_at_endpoints() {
        let p = Parameter::new("gain", -6.0, 6.0, 0.0);
        assert_eq!(p.denormalize(0.0), -6.0);
        assert_eq!(p.denormalize(1.0), 6.0);
        assert_eq!(p.denormalize(0.5), 0.0);
    }

    #[test]
    fn zero_width_range_normalizes_to_zero() {
        let p = Parameter::new("fixed", 1.0, 1.0, 1.0);
        assert_eq!(p.normalize(1.0), 0.0);
    }
}
