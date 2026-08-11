//! `vb-standalone` — standalone app (cpal + editor).
//!
//! Stub binary; the real standalone app lands in Epic 6.

/// Returns this binary's name. Placeholder until the real app lands.
#[must_use]
fn app_name() -> &'static str {
    "vb-standalone"
}

fn main() {
    println!("{} (stub)", app_name());
}

#[cfg(test)]
mod tests {
    use super::app_name;

    #[test]
    fn exposes_app_name() {
        assert_eq!(app_name(), "vb-standalone");
    }
}
