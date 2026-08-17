//! A modulation matrix: named sources (an LFO, an envelope) routed to parameter
//! destinations with a per-route depth.
//!
//! Destinations are graph parameters addressed by their stable [`ParamId`] (see the
//! mapping system, 000076); a source is a named signal whose current value the host
//! supplies each block. A route contributes `amount * source_value` to its destination,
//! and a destination's modulated value is its base plus the sum of its routes, clamped
//! into the parameter's range. The matrix is plain, serializable data — the routing
//! table a UI edits and the plugin persists; evaluating it against live source values is
//! a pure function.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vb_core::Parameter;

use crate::mapping::ParamId;

/// Current modulation-matrix format version.
pub const MODMATRIX_VERSION: u32 = 1;

/// A named modulation source (e.g. `"lfo1"`, `"env1"`) whose value the host supplies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModSource(pub String);

impl ModSource {
    /// Creates a source with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// One route: `source` drives `dest` with signed depth `amount`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModRoute {
    /// The modulation source.
    pub source: ModSource,
    /// The destination parameter.
    pub dest: ParamId,
    /// Depth: the destination gains `amount * source_value`.
    pub amount: f32,
}

/// What can go wrong reading a matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModMatrixError {
    /// RON serialization failed.
    Serialize(String),
    /// RON parsing failed.
    Deserialize(String),
    /// The file's version is newer than this build supports.
    UnsupportedVersion(u32),
}

impl std::fmt::Display for ModMatrixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModMatrixError::Serialize(e) => write!(f, "serialize modulation matrix: {e}"),
            ModMatrixError::Deserialize(e) => write!(f, "parse modulation matrix: {e}"),
            ModMatrixError::UnsupportedVersion(v) => write!(
                f,
                "modulation-matrix version {v} is newer than supported {MODMATRIX_VERSION}"
            ),
        }
    }
}

impl std::error::Error for ModMatrixError {}

/// A routing table of modulation sources to parameter destinations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModMatrix {
    /// Format version (see [`MODMATRIX_VERSION`]).
    pub version: u32,
    /// The routes, in table order.
    routes: Vec<ModRoute>,
}

impl Default for ModMatrix {
    fn default() -> Self {
        Self {
            version: MODMATRIX_VERSION,
            routes: Vec::new(),
        }
    }
}

impl ModMatrix {
    /// An empty matrix.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a route and returns its index.
    pub fn add(&mut self, route: ModRoute) -> usize {
        self.routes.push(route);
        self.routes.len() - 1
    }

    /// Removes the route at `index`, returning it if present.
    pub fn remove(&mut self, index: usize) -> Option<ModRoute> {
        (index < self.routes.len()).then(|| self.routes.remove(index))
    }

    /// All routes, in table order.
    #[must_use]
    pub fn routes(&self) -> &[ModRoute] {
        &self.routes
    }

    /// Number of routes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Whether the matrix has no routes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// The distinct source names referenced by the routes, in first-seen order.
    #[must_use]
    pub fn sources(&self) -> Vec<&ModSource> {
        let mut seen = Vec::new();
        for route in &self.routes {
            if !seen.contains(&&route.source) {
                seen.push(&route.source);
            }
        }
        seen
    }

    /// The total modulation offset applied to `dest`: the sum of `amount * source_value`
    /// over its routes. A source with no supplied value contributes nothing.
    #[must_use]
    pub fn offset(&self, dest: &ParamId, sources: &HashMap<ModSource, f32>) -> f32 {
        self.routes
            .iter()
            .filter(|r| &r.dest == dest)
            .map(|r| r.amount * sources.get(&r.source).copied().unwrap_or(0.0))
            .sum()
    }

    /// The modulated value of `dest`: `base` plus its [`offset`](Self::offset), clamped
    /// into `param`'s range.
    #[must_use]
    pub fn modulate(
        &self,
        dest: &ParamId,
        base: f32,
        param: &Parameter,
        sources: &HashMap<ModSource, f32>,
    ) -> f32 {
        param.clamp(base + self.offset(dest, sources))
    }

    /// Serializes to a pretty RON string.
    ///
    /// # Errors
    /// [`ModMatrixError::Serialize`] if RON serialization fails.
    pub fn to_ron(&self) -> Result<String, ModMatrixError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| ModMatrixError::Serialize(e.to_string()))
    }

    /// Parses a matrix from RON, rejecting a future version.
    ///
    /// # Errors
    /// [`ModMatrixError::Deserialize`] on malformed RON, or
    /// [`ModMatrixError::UnsupportedVersion`].
    pub fn from_ron(text: &str) -> Result<Self, ModMatrixError> {
        let matrix: Self =
            ron::from_str(text).map_err(|e| ModMatrixError::Deserialize(e.to_string()))?;
        if matrix.version > MODMATRIX_VERSION {
            return Err(ModMatrixError::UnsupportedVersion(matrix.version));
        }
        Ok(matrix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_core::NodeId;

    fn route(source: &str, dest: ParamId, amount: f32) -> ModRoute {
        ModRoute {
            source: ModSource::new(source),
            dest,
            amount,
        }
    }

    fn sources(pairs: &[(&str, f32)]) -> HashMap<ModSource, f32> {
        pairs
            .iter()
            .map(|(n, v)| (ModSource::new(*n), *v))
            .collect()
    }

    #[test]
    fn add_and_remove_routes() {
        let mut m = ModMatrix::new();
        assert!(m.is_empty());
        let cutoff = ParamId::of(NodeId(1), 0);
        m.add(route("lfo1", cutoff.clone(), 0.5));
        m.add(route("env1", cutoff.clone(), 0.25));
        assert_eq!(m.len(), 2);
        let removed = m.remove(0).unwrap();
        assert_eq!(removed.source, ModSource::new("lfo1"));
        assert_eq!(m.len(), 1);
        assert!(m.remove(9).is_none());
    }

    #[test]
    fn offset_sums_routes_to_the_destination_only() {
        let mut m = ModMatrix::new();
        let cutoff = ParamId::of(NodeId(1), 0);
        let gain = ParamId::of(NodeId(2), 0);
        m.add(route("lfo1", cutoff.clone(), 100.0));
        m.add(route("env1", cutoff.clone(), 200.0));
        m.add(route("lfo1", gain.clone(), 0.5));

        let src = sources(&[("lfo1", 1.0), ("env1", 0.5)]);
        // cutoff: 100*1.0 + 200*0.5 = 200; gain: 0.5*1.0 = 0.5.
        assert!((m.offset(&cutoff, &src) - 200.0).abs() < 1e-4);
        assert!((m.offset(&gain, &src) - 0.5).abs() < 1e-6);
        // A destination with no routes gets nothing.
        assert_eq!(m.offset(&ParamId::of(NodeId(9), 0), &src), 0.0);
    }

    #[test]
    fn a_missing_source_value_contributes_zero() {
        let mut m = ModMatrix::new();
        let dest = ParamId::of(NodeId(1), 0);
        m.add(route("lfo1", dest.clone(), 1.0));
        m.add(route("absent", dest.clone(), 1.0));
        // Only lfo1 has a value; "absent" is treated as 0.
        assert_eq!(m.offset(&dest, &sources(&[("lfo1", 0.3)])), 0.3);
    }

    #[test]
    fn modulate_adds_the_offset_and_clamps_into_range() {
        let mut m = ModMatrix::new();
        let cutoff = ParamId::of(NodeId(1), 0);
        let p = Parameter::new("cutoff_hz", 20.0, 20_000.0, 1_000.0);
        m.add(route("lfo1", cutoff.clone(), 5_000.0));

        // base 1000 + 5000*1.0 = 6000, in range.
        assert!(
            (m.modulate(&cutoff, 1_000.0, &p, &sources(&[("lfo1", 1.0)])) - 6_000.0).abs() < 1e-2
        );
        // Pushed past the max → clamped.
        assert_eq!(
            m.modulate(&cutoff, 19_000.0, &p, &sources(&[("lfo1", 1.0)])),
            20_000.0
        );
        // Negative depth pushes below the min → clamped up.
        m.add(route("lfo1", cutoff.clone(), -100_000.0));
        assert_eq!(
            m.modulate(&cutoff, 1_000.0, &p, &sources(&[("lfo1", 1.0)])),
            20.0
        );
    }

    #[test]
    fn sources_lists_distinct_names_in_first_seen_order() {
        let mut m = ModMatrix::new();
        let d = ParamId::of(NodeId(1), 0);
        m.add(route("lfo1", d.clone(), 1.0));
        m.add(route("env1", d.clone(), 1.0));
        m.add(route("lfo1", d.clone(), 0.5)); // duplicate source
        let names: Vec<&str> = m.sources().iter().map(|s| s.0.as_str()).collect();
        assert_eq!(names, vec!["lfo1", "env1"]);
    }

    #[test]
    fn round_trips_through_ron() {
        let mut m = ModMatrix::new();
        m.add(route("lfo1", ParamId::of(NodeId(1), 0), 0.75));
        let ron = m.to_ron().unwrap();
        assert_eq!(ModMatrix::from_ron(&ron).unwrap(), m);

        let mut future = m.clone();
        future.version = MODMATRIX_VERSION + 1;
        assert_eq!(
            ModMatrix::from_ron(&future.to_ron().unwrap()),
            Err(ModMatrixError::UnsupportedVersion(MODMATRIX_VERSION + 1))
        );
    }
}
