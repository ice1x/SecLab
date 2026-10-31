//! Shared short-time Fourier transform (STFT) overlap-add engine for the spectral nodes.
//!
//! [`crate::PitchShifter`] and [`crate::SpectralFreeze`] (and any future spectral effect) share
//! the same scaffolding: a Hann-windowed analysis FIFO, a per-hop forward FFT, an inverse FFT
//! with constant-overlap-add resynthesis, and a latency-matched dry delay. Only the *spectral
//! transform* in the middle — what each node does to the bins between analysis and synthesis —
//! differs. That shared machinery lives here so each node only writes its own transform.
//!
//! Usage from a node's `process` loop, per sample:
//! ```ignore
//! let dry = self.stft.dry_delay(x);
//! let (wet, frame_ready) = self.stft.push(x);
//! if frame_ready {
//!     self.stft.forward();   // window + forward FFT into `self.stft.fft_work`
//!     self.transform();      // node-specific: read/modify `self.stft.fft_work[0..=HALF]`
//!     self.stft.synthesize();// inverse FFT + windowed overlap-add, fills the output FIFO
//! }
//! out = mix * wet + (1.0 - mix) * dry;
//! ```
//!
//! The FFT follows the `smbFft` convention via [`crate::fft`] (`sign -1` forward, `+1`
//! unnormalised inverse). The resynthesis normalisation is Bernsee's `2/(HALF·osamp)` times the
//! `2/3` Hann² constant-overlap-add gain (the window is applied at both analysis and synthesis),
//! which reconstructs unit gain when the transform is the identity.

use crate::fft::fft;

/// FFT frame size (number of real samples per frame). A power of two.
pub(crate) const FFT_SIZE: usize = 1024;
/// Oversampling / overlap factor (75 % overlap at 4).
pub(crate) const OSAMP: usize = 4;
/// Hop between successive frames.
pub(crate) const STEP_SIZE: usize = FFT_SIZE / OSAMP;
/// Number of positive-frequency bins (DC … Nyquist inclusive).
pub(crate) const HALF: usize = FFT_SIZE / 2;
/// Samples the FIFO fills before the first frame; also the engine's latency.
pub(crate) const LATENCY: usize = FFT_SIZE - STEP_SIZE;

/// The shared STFT analysis/synthesis state: window, FIFOs, FFT workspace, overlap accumulator,
/// and the latency-matched dry delay.
pub(crate) struct Stft {
    window: Vec<f32>,
    in_fifo: Vec<f32>,
    out_fifo: Vec<f32>,
    /// Interleaved complex FFT workspace (`2·FFT_SIZE`); the node's transform reads and writes it.
    pub(crate) fft_work: Vec<f32>,
    out_accum: Vec<f32>,
    rover: usize,
    dry: Vec<f32>,
    dry_idx: usize,
}

impl Stft {
    pub(crate) fn new() -> Self {
        let window = (0..FFT_SIZE)
            .map(|k| 0.5 - 0.5 * (core::f32::consts::TAU * k as f32 / FFT_SIZE as f32).cos())
            .collect();
        Self {
            window,
            in_fifo: vec![0.0; FFT_SIZE],
            out_fifo: vec![0.0; FFT_SIZE],
            fft_work: vec![0.0; 2 * FFT_SIZE],
            out_accum: vec![0.0; 2 * FFT_SIZE],
            rover: LATENCY,
            dry: vec![0.0; LATENCY],
            dry_idx: 0,
        }
    }

    /// Pushes `x` through the latency-matched dry delay and returns the delayed dry sample, so a
    /// node's `mix` dry path lines up with its latent wet output.
    pub(crate) fn dry_delay(&mut self, x: f32) -> f32 {
        let d = self.dry[self.dry_idx];
        self.dry[self.dry_idx] = x;
        self.dry_idx = (self.dry_idx + 1) % LATENCY;
        d
    }

    /// Writes `x` into the analysis FIFO and reads the matching output sample. Returns
    /// `(wet, frame_ready)`; when `frame_ready` is true the caller must run
    /// [`Stft::forward`] → its transform → [`Stft::synthesize`].
    pub(crate) fn push(&mut self, x: f32) -> (f32, bool) {
        self.in_fifo[self.rover] = x;
        let wet = self.out_fifo[self.rover - LATENCY];
        self.rover += 1;
        if self.rover >= FFT_SIZE {
            self.rover = LATENCY;
            (wet, true)
        } else {
            (wet, false)
        }
    }

    /// Windows the current frame into `fft_work` and runs the forward FFT.
    pub(crate) fn forward(&mut self) {
        for k in 0..FFT_SIZE {
            self.fft_work[2 * k] = self.in_fifo[k] * self.window[k];
            self.fft_work[2 * k + 1] = 0.0;
        }
        fft(&mut self.fft_work, FFT_SIZE, -1.0);
    }

    /// Zeroes the negative-frequency bins, runs the inverse FFT, and overlap-adds the windowed,
    /// normalised result into the output FIFO (then shifts the accumulator and input FIFO by one
    /// hop). Call after the node has written its synthesised half-spectrum into `fft_work`.
    pub(crate) fn synthesize(&mut self) {
        for k in (HALF + 1)..FFT_SIZE {
            self.fft_work[2 * k] = 0.0;
            self.fft_work[2 * k + 1] = 0.0;
        }
        fft(&mut self.fft_work, FFT_SIZE, 1.0);

        let norm = 2.0 / (HALF as f32 * OSAMP as f32) * (2.0 / 3.0);
        for k in 0..FFT_SIZE {
            self.out_accum[k] += self.window[k] * self.fft_work[2 * k] * norm;
        }
        self.out_fifo[..STEP_SIZE].copy_from_slice(&self.out_accum[..STEP_SIZE]);
        self.out_accum
            .copy_within(STEP_SIZE..STEP_SIZE + FFT_SIZE, 0);
        self.in_fifo.copy_within(STEP_SIZE..FFT_SIZE, 0);
    }

    /// Clears all FIFOs, the accumulator, the dry delay and the cursor.
    pub(crate) fn reset(&mut self) {
        self.in_fifo.iter_mut().for_each(|s| *s = 0.0);
        self.out_fifo.iter_mut().for_each(|s| *s = 0.0);
        self.out_accum.iter_mut().for_each(|s| *s = 0.0);
        self.dry.iter_mut().for_each(|s| *s = 0.0);
        self.rover = LATENCY;
        self.dry_idx = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_transform_reconstructs_a_tone_at_unit_gain() {
        // With no spectral change between forward() and synthesize(), the engine is a pure
        // analysis/resynthesis round-trip: a tone comes back at the same amplitude.
        let mut s = Stft::new();
        let n = 16_384;
        let input: Vec<f32> = (0..n)
            .map(|i| (core::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        let mut out = vec![0.0_f32; n];
        for i in 0..n {
            let (wet, ready) = s.push(input[i]);
            if ready {
                s.forward();
                // Faithful real-signal identity: keep the positive half but double it (the node
                // convention, since `synthesize` drops the negative half without compensating).
                for k in 0..=HALF {
                    s.fft_work[2 * k] *= 2.0;
                    s.fft_work[2 * k + 1] *= 2.0;
                }
                s.synthesize();
            }
            out[i] = wet;
        }
        // Measure the settled tail at 440 Hz (Goertzel), expect ≈ 0.5 for a unit sine.
        let w = core::f32::consts::TAU * 440.0 / 48_000.0;
        let (mut re, mut im) = (0.0_f32, 0.0_f32);
        for (k, &x) in out[8_192..].iter().enumerate() {
            re += x * (w * k as f32).cos();
            im -= x * (w * k as f32).sin();
        }
        let m = (re * re + im * im).sqrt() / out[8_192..].len() as f32;
        assert!(
            (m - 0.5).abs() < 0.08,
            "identity round-trip is unit gain, got {m}"
        );
    }

    #[test]
    fn latency_relationship_holds() {
        assert_eq!(LATENCY, FFT_SIZE - STEP_SIZE);
        assert_eq!(HALF, FFT_SIZE / 2);
        assert_eq!(STEP_SIZE, FFT_SIZE / OSAMP);
    }
}
