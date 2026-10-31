//! Audio settings choices: which sample rates and block sizes to offer in the UI.
//!
//! Pure logic over plain numbers — device enumeration stays in `main.rs`, so this is
//! unit-testable without any audio hardware.

/// Engine block sizes offered in the UI, in frames.
pub const BLOCK_OPTIONS: &[usize] = &[64, 128, 256, 512, 1024];

/// Sample rates worth offering, when the device supports them.
pub const RATE_CANDIDATES: &[u32] = &[44_100, 48_000, 88_200, 96_000, 192_000];

/// The candidates from [`RATE_CANDIDATES`] that fall inside any of the device's
/// supported `(min, max)` ranges, ascending and deduplicated.
///
/// An empty result means the device supports none of the common rates — the caller
/// should fall back to the device's default configuration.
#[must_use]
pub fn rates_within(ranges: &[(u32, u32)]) -> Vec<u32> {
    let mut rates: Vec<u32> = RATE_CANDIDATES
        .iter()
        .copied()
        .filter(|rate| ranges.iter().any(|(min, max)| (min..=max).contains(&rate)))
        .collect();
    rates.dedup();
    rates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_candidates_inside_the_ranges() {
        // A typical mac output: one wide range.
        assert_eq!(
            rates_within(&[(44_100, 96_000)]),
            vec![44_100, 48_000, 88_200, 96_000]
        );
    }

    #[test]
    fn unions_disjoint_ranges() {
        // Some devices report one range per discrete rate.
        assert_eq!(
            rates_within(&[(44_100, 44_100), (96_000, 96_000)]),
            vec![44_100, 96_000]
        );
    }

    #[test]
    fn no_overlap_yields_empty() {
        assert!(rates_within(&[(8_000, 16_000)]).is_empty());
        assert!(rates_within(&[]).is_empty());
    }

    #[test]
    fn block_options_are_sane_powers_of_two() {
        for w in BLOCK_OPTIONS.windows(2) {
            assert!(w[0] < w[1], "ascending");
        }
        assert!(BLOCK_OPTIONS.iter().all(|b| b.is_power_of_two()));
    }
}
