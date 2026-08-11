//! `vb-project` — project serialization (graph format, save/load, versions).
//!
//! Stub crate; the real implementation lands in Epic 5.

/// Returns this crate's name. Placeholder until the real API lands.
#[must_use]
pub fn crate_name() -> &'static str {
    "vb-project"
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "vb-project");
    }
}
