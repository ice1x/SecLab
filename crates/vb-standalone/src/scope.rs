//! Live signal visualization: a lock-free sample tap plus pure analysis.
//!
//! The audio thread pushes each played block into a [`Scope`]'s ring; the UI thread
//! snapshots the most recent samples and derives a peak meter, an oscilloscope trace,
//! and a coarse spectrum. The write side is allocation-free and lock-free (one
//! atomic store per sample plus one index bump); the read side may tear by a sample
//! or two, which is invisible in a meter refreshed ~30x/second.
//!
//! The analysis functions are pure and unit-tested — no audio hardware, no atomics.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

/// Number of samples retained for the scope/spectrum (~21 ms at 48 kHz).
pub const SCOPE_LEN: usize = 1024;

/// Frequency bands (Hz) the coarse spectrum reports, one magnitude each.
pub const SPECTRUM_BANDS: [f32; 8] = [
    60.0, 150.0, 400.0, 1_000.0, 2_400.0, 6_000.0, 12_000.0, 18_000.0,
];

/// Lock-free ring the audio thread writes and the UI thread snapshots.
#[derive(Clone, Default)]
pub struct Scope {
    inner: Arc<ScopeShared>,
}

struct ScopeShared {
    /// Samples as bit patterns; a power-of-two ring.
    ring: Vec<AtomicU32>,
    /// Total samples written; `write % len` is the next slot.
    write: AtomicUsize,
}

impl Default for ScopeShared {
    fn default() -> Self {
        Self {
            ring: (0..SCOPE_LEN).map(|_| AtomicU32::new(0)).collect(),
            write: AtomicUsize::new(0),
        }
    }
}

impl Scope {
    /// Creates an empty scope.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a block of samples (audio-thread side, allocation-free).
    pub fn push(&self, block: &[f32]) {
        let len = self.inner.ring.len();
        let mut w = self.inner.write.load(Ordering::Relaxed);
        for &s in block {
            self.inner.ring[w % len].store(s.to_bits(), Ordering::Relaxed);
            w += 1;
        }
        self.inner.write.store(w, Ordering::Release);
    }

    /// Copies the most recent [`SCOPE_LEN`] samples into `out`, oldest first.
    ///
    /// `out` is caller-owned so the UI can reuse one buffer across frames.
    pub fn snapshot(&self, out: &mut [f32; SCOPE_LEN]) {
        let len = self.inner.ring.len();
        let w = self.inner.write.load(Ordering::Acquire);
        for (i, slot) in out.iter_mut().enumerate() {
            // The oldest retained sample sits `len` behind the write head.
            let idx = (w + i) % len;
            *slot = f32::from_bits(self.inner.ring[idx].load(Ordering::Relaxed));
        }
    }
}

/// Peak absolute amplitude of `samples` (0 for an empty slice).
#[must_use]
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

/// Root-mean-square level of `samples` (0 for an empty slice).
#[must_use]
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Goertzel magnitude of `samples` at `freq_hz` — the energy in one frequency bin
/// without a full FFT. Normalized so a unit sine at `freq_hz` reads ~1.0.
#[must_use]
pub fn goertzel(samples: &[f32], freq_hz: f32, sample_rate: f32) -> f32 {
    let n = samples.len();
    if n == 0 || sample_rate <= 0.0 {
        return 0.0;
    }
    let omega = std::f32::consts::TAU * freq_hz / sample_rate;
    let coeff = 2.0 * omega.cos();
    let (mut s_prev, mut s_prev2) = (0.0_f32, 0.0_f32);
    for &x in samples {
        let s = coeff.mul_add(s_prev, x) - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    let power = s_prev2.mul_add(s_prev2, s_prev * s_prev) - coeff * s_prev * s_prev2;
    // 2/N normalization gives ~unit amplitude for an on-bin unit sine.
    power.max(0.0).sqrt() * 2.0 / n as f32
}

/// Magnitudes at each [`SPECTRUM_BANDS`] frequency, clamped to `[0, 1]`.
#[must_use]
pub fn spectrum(samples: &[f32], sample_rate: f32) -> [f32; SPECTRUM_BANDS.len()] {
    let mut out = [0.0_f32; SPECTRUM_BANDS.len()];
    for (o, &freq) in out.iter_mut().zip(&SPECTRUM_BANDS) {
        *o = goertzel(samples, freq, sample_rate).min(1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(freq: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin())
            .collect()
    }

    #[test]
    fn peak_and_rms_of_a_unit_sine() {
        let s = sine(1_000.0, 4_800);
        assert!((peak(&s) - 1.0).abs() < 0.01);
        // RMS of a sine is 1/sqrt(2).
        assert!((rms(&s) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn goertzel_peaks_at_the_signal_frequency() {
        let s = sine(1_000.0, 2_048);
        let on = goertzel(&s, 1_000.0, SR);
        let off = goertzel(&s, 5_000.0, SR);
        assert!((on - 1.0).abs() < 0.1, "on-bin ≈ 1.0, got {on}");
        assert!(off < 0.2, "off-bin small, got {off}");
        assert_eq!(goertzel(&[], 1_000.0, SR), 0.0);
        assert_eq!(goertzel(&s, 1_000.0, 0.0), 0.0);
    }

    #[test]
    fn spectrum_lights_the_band_nearest_the_tone() {
        // A 1 kHz tone should peak the 1000 Hz band above its neighbours.
        let s = sine(1_000.0, SCOPE_LEN);
        let bands = spectrum(&s, SR);
        let peak_band = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(SPECTRUM_BANDS[peak_band], 1_000.0);
        assert!(bands.iter().all(|&m| (0.0..=1.0).contains(&m)));
    }

    #[test]
    fn scope_snapshot_returns_the_most_recent_samples() {
        let scope = Scope::new();
        // Push a ramp longer than the ring so it wraps.
        let ramp: Vec<f32> = (0..SCOPE_LEN + 100).map(|i| i as f32).collect();
        scope.push(&ramp);

        let mut out = [0.0_f32; SCOPE_LEN];
        scope.snapshot(&mut out);
        // Newest sample is the last pushed; oldest is SCOPE_LEN behind it.
        assert_eq!(out[SCOPE_LEN - 1], (SCOPE_LEN + 100 - 1) as f32);
        assert_eq!(out[0], 100.0);
        // Monotonic — no torn ordering for a single-threaded push.
        assert!(out.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn empty_scope_snapshots_silence() {
        let scope = Scope::new();
        let mut out = [9.0_f32; SCOPE_LEN];
        scope.snapshot(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }
}
