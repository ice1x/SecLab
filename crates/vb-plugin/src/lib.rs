//! `vb-plugin` — CLAP export (nih-plug), VST3/AU build via clap-wrapper.
//!
//! Stub crate; the real implementation lands in Epic 7.

/// Returns this crate's name. Placeholder until the real API lands.
#[must_use]
pub fn crate_name() -> &'static str {
    "vb-plugin"
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "vb-plugin");
    }
}
