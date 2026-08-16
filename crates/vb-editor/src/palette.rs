//! The node palette: search the registry and list what can be added to the canvas.
//!
//! Pure filtering over [`vb_nodes::Registry`] — the egui side panel just renders what
//! [`palette_entries`] returns, so matching and ordering are unit-testable.

use vb_nodes::Registry;

/// One palette row: a spawnable node kind with its category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    /// Registry kind name (e.g. `"oscillator"`).
    pub kind: &'static str,
    /// UI category (e.g. `"generator"`), used as the group header.
    pub category: &'static str,
}

/// Registry kinds matching `query`, sorted by category then kind.
///
/// Matching is a case-insensitive substring test against the kind *or* the category,
/// so `"osc"` finds the oscillator and `"math"` lists the whole math group. An empty
/// (or all-whitespace) query lists everything.
#[must_use]
pub fn palette_entries(registry: &Registry, query: &str) -> Vec<PaletteEntry> {
    let needle = query.trim().to_lowercase();
    let mut entries: Vec<PaletteEntry> = registry
        .kinds()
        .filter_map(|kind| registry.get(kind))
        .map(|d| PaletteEntry {
            kind: d.kind(),
            category: d.category(),
        })
        .filter(|e| {
            needle.is_empty()
                || e.kind.to_lowercase().contains(&needle)
                || e.category.to_lowercase().contains(&needle)
        })
        .collect();
    entries.sort_by_key(|e| (e.category, e.kind));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_lists_every_builtin_grouped_by_category() {
        let registry = Registry::with_builtins();
        let entries = palette_entries(&registry, "");
        assert_eq!(entries.len(), registry.kinds().count());
        // Sorted by (category, kind): categories arrive in runs, kinds sorted inside.
        let sorted = {
            let mut copy = entries.clone();
            copy.sort_by_key(|e| (e.category, e.kind));
            copy
        };
        assert_eq!(entries, sorted);
        // Whitespace-only behaves like empty.
        assert_eq!(palette_entries(&registry, "   "), entries);
    }

    #[test]
    fn kind_substring_matches_case_insensitively() {
        let registry = Registry::with_builtins();
        let entries = palette_entries(&registry, "OSC");
        assert!(entries.iter().any(|e| e.kind == "oscillator"));
        assert!(entries.iter().all(|e| e.kind.contains("osc")));
    }

    #[test]
    fn category_query_lists_the_whole_group() {
        let registry = Registry::with_builtins();
        let entries = palette_entries(&registry, "math");
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.category == "math"));
        // The math group has more than one node — the category match is the point.
        assert!(entries.len() > 1);
    }

    #[test]
    fn garbage_query_matches_nothing() {
        let registry = Registry::with_builtins();
        assert!(palette_entries(&registry, "zzzznope").is_empty());
    }
}
