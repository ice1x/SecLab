//! Channel vocoder: imprints the spectral envelope of a *modulator* onto a *carrier*.
//!
//! The classic "robot voice" effect (Dudley's voder/vocoder, the Moog and EMS hardware
//! vocoders): the modulator (typically a voice) is split into a bank of band-pass filters and
//! each band's level is tracked by an envelope follower; the carrier (typically a synth) is split
//! by the **same** filter bank, and each carrier band is multiplied by the corresponding
//! modulator band's envelope before the bands are summed. The carrier ends up carrying the
//! moment-to-moment spectral shape of the modulator, so a sustained synth chord "speaks" the
//! words of the voice.
//!
//! Input 0 is the **carrier** (shaped), input 1 is the **modulator** (the voice/control).
//! Output 0 is the vocoded signal.
//!
//! Band plan: [`BANDS`] log-spaced constant-Q band-pass bands from [`F_LO`] to [`F_HI`]. The
//! number of bands is the dominant quality factor; 16 follows the Moog 16-channel vocoder (which
//! spans 50–5080 Hz), and the logarithmic spacing matches pitch perception (the MFOS 12-channel
//! design spaces its bands at a constant ≈1.35× ratio). The exact plan is a design choice, not a
//! fixed constant — the vocoder *law* (carrier bands gated by the modulator's per-band envelope)
//! holds for any reasonable bank. Each band is an RBJ band-pass biquad run on the shared
//! [`crate::biquad`] Direct Form I core; the per-band `Q` is derived from the band spacing so the
//! bands tile the spectrum.
//!
//! Primary sources: Wikipedia "Vocoder"; Moog 16-channel vocoder (16 bands, 50–5080 Hz); MFOS
//! 12-channel vocoder (log-spaced band table). RBJ Audio EQ Cookbook band-pass biquad.

use crate::biquad::{BiquadCoeffs, BiquadState};
use seclab_core::{ProcessContext, Processor};

/// Number of filter bands.
pub const BANDS: usize = 16;
/// Lowest band centre (Hz).
pub const F_LO: f32 = 80.0;
/// Highest band centre (Hz).
pub const F_HI: f32 = 7_000.0;

/// RBJ band-pass biquad (constant skirt gain, peak gain = `Q`) at `f0` with resonance `q`.
fn band_pass(f0: f32, q: f32, sample_rate: f32) -> BiquadCoeffs {
    let f0 = f0.clamp(1.0, 0.49 * sample_rate);
    let w0 = core::f32::consts::TAU * (f0 / sample_rate);
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q.max(1e-4));
    BiquadCoeffs::normalized(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
}

/// A channel vocoder.
pub struct Vocoder {
    release_ms: f32,
    mix: f32,

    /// Band centre frequencies (log-spaced), precomputed.
    centers: [f32; BANDS],
    /// Constant per-band Q, derived from the band spacing.
    q: f32,

    /// Modulator analysis filters.
    mod_bp: [BiquadState; BANDS],
    /// Carrier synthesis filters (same centres/Q as the modulator bank).
    car_bp: [BiquadState; BANDS],
    /// Per-band envelope-follower state.
    env: [f32; BANDS],
}

impl Vocoder {
    /// Creates a vocoder with envelope `release_ms` (follower smoothing) and dry/wet `mix`
    /// (0 = dry carrier, 1 = fully vocoded).
    #[must_use]
    pub fn new(release_ms: f32, mix: f32) -> Self {
        // Log-spaced centres: f_k = F_LO · r^k with r = (F_HI/F_LO)^(1/(BANDS−1)).
        let ratio = (F_HI / F_LO).powf(1.0 / (BANDS - 1) as f32);
        let mut centers = [0.0_f32; BANDS];
        for (k, c) in centers.iter_mut().enumerate() {
            *c = F_LO * ratio.powi(k as i32);
        }
        // Tile the bands: set the −3 dB fractional bandwidth to one band step, so Q = 1/(√r−1/√r).
        let sr = ratio.sqrt();
        let q = 1.0 / (sr - 1.0 / sr);
        Self {
            release_ms: release_ms.clamp(1.0, 500.0),
            mix: mix.clamp(0.0, 1.0),
            centers,
            q,
            mod_bp: [BiquadState::default(); BANDS],
            car_bp: [BiquadState::default(); BANDS],
            env: [0.0; BANDS],
        }
    }
}

impl Processor for Vocoder {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let carrier = inputs.first().copied().unwrap_or(&[]);
        let modulator = inputs.get(1).copied().unwrap_or(&[]);
        if ctx.sample_rate <= 0.0 {
            for (o, c) in outputs[0].iter_mut().zip(carrier.iter()).take(ctx.frames) {
                *o = *c;
            }
            return;
        }

        // Per-block coefficients and the envelope-follower one-pole coefficient.
        let mut coeffs = [BiquadCoeffs::normalized(1.0, 0.0, 0.0, 1.0, 0.0, 0.0); BANDS];
        for (b, c) in coeffs.iter_mut().enumerate() {
            *c = band_pass(self.centers[b], self.q, ctx.sample_rate);
        }
        let tau = (self.release_ms / 1_000.0).max(1e-4);
        let env_coeff = 1.0 - (-1.0 / (tau * ctx.sample_rate)).exp();
        let mix = self.mix;

        for (i, o) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let c = carrier.get(i).copied().unwrap_or(0.0);
            let m = modulator.get(i).copied().unwrap_or(0.0);
            let mut acc = 0.0_f32;
            for (((coeff, mod_st), car_st), env) in coeffs
                .iter()
                .zip(self.mod_bp.iter_mut())
                .zip(self.car_bp.iter_mut())
                .zip(self.env.iter_mut())
            {
                // Modulator band → rectified → one-pole envelope.
                let mo = mod_st.step(coeff, m);
                *env += env_coeff * (mo.abs() - *env);
                // Carrier band gated by that envelope.
                let co = car_st.step(coeff, c);
                acc += co * *env;
            }
            *o = mix * acc + (1.0 - mix) * c;
        }
    }

    fn reset(&mut self) {
        self.mod_bp = [BiquadState::default(); BANDS];
        self.car_bp = [BiquadState::default(); BANDS];
        self.env = [0.0; BANDS];
    }

    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&r) = values.first() {
            self.release_ms = r.clamp(1.0, 500.0);
        }
        if let Some(&m) = values.get(1) {
            self.mix = m.clamp(0.0, 1.0);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(p: &mut Vocoder, carrier: &[f32], modulator: &[f32]) -> Vec<f32> {
        let n = carrier.len().max(modulator.len());
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
        };
        let mut out = vec![0.0_f32; n];
        {
            let ins: [&[f32]; 2] = [carrier, modulator];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            p.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn tone(hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * hz * i as f32 / SR).sin())
            .collect()
    }

    /// Deterministic white-ish noise in [−1, 1] from a small LCG.
    fn noise(n: usize) -> Vec<f32> {
        let mut state = 0x1234_5678_u32;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0
            })
            .collect()
    }

    fn mag(xs: &[f32], hz: f32) -> f32 {
        let w = core::f32::consts::TAU * hz / SR;
        let (mut re, mut im) = (0.0_f32, 0.0_f32);
        for (n, &x) in xs.iter().enumerate() {
            re += x * (w * n as f32).cos();
            im -= x * (w * n as f32).sin();
        }
        (re * re + im * im).sqrt() / xs.len() as f32
    }

    #[test]
    fn centers_are_log_spaced_across_the_band_range() {
        let v = Vocoder::new(15.0, 1.0);
        assert!((v.centers[0] - F_LO).abs() < 1e-3);
        assert!((v.centers[BANDS - 1] - F_HI).abs() < 1.0);
        // Equal ratios between neighbours (logarithmic spacing).
        let r0 = v.centers[1] / v.centers[0];
        let r1 = v.centers[BANDS - 1] / v.centers[BANDS - 2];
        assert!((r0 - r1).abs() < 1e-3, "constant ratio {r0} vs {r1}");
    }

    #[test]
    fn a_silent_modulator_gives_a_silent_output() {
        // No modulator energy ⇒ every band envelope stays at zero ⇒ the carrier is fully gated.
        let out = run(
            &mut Vocoder::new(15.0, 1.0),
            &noise(8_192),
            &vec![0.0; 8_192],
        );
        assert!(
            out.iter().all(|s| s.abs() < 1e-4),
            "a silent modulator mutes the carrier"
        );
    }

    #[test]
    fn a_silent_carrier_gives_a_silent_output() {
        // Nothing to shape ⇒ silence, however loud the modulator.
        let out = run(
            &mut Vocoder::new(15.0, 1.0),
            &vec![0.0; 8_192],
            &tone(1_000.0, 8_192),
        );
        assert!(
            out.iter().all(|s| s.abs() < 1e-4),
            "a silent carrier produces silence"
        );
    }

    #[test]
    fn the_output_takes_the_modulators_band_from_a_flat_carrier() {
        // A broadband (noise) carrier gated by a modulator tone near 1.2 kHz comes out with its
        // energy concentrated near 1.2 kHz, not down at 260 Hz.
        let n = 32_768;
        let out = run(&mut Vocoder::new(15.0, 1.0), &noise(n), &tone(1_200.0, n));
        let tail = &out[16_384..];
        let (hi, lo) = (mag(tail, 1_200.0), mag(tail, 260.0));
        assert!(
            hi > 3.0 * lo,
            "energy sits in the modulator's band: hi {hi}, lo {lo}"
        );
    }

    #[test]
    fn moving_the_modulator_band_moves_the_output_band() {
        // The mirror case: a low modulator tone pushes the carrier energy down to the low band.
        let n = 32_768;
        let out = run(&mut Vocoder::new(15.0, 1.0), &noise(n), &tone(260.0, n));
        let tail = &out[16_384..];
        let (hi, lo) = (mag(tail, 1_200.0), mag(tail, 260.0));
        assert!(
            lo > 3.0 * hi,
            "energy tracks down to the low band: lo {lo}, hi {hi}"
        );
    }

    #[test]
    fn output_stays_finite_and_bounded() {
        let n = 16_384;
        let out = run(&mut Vocoder::new(15.0, 1.0), &noise(n), &tone(500.0, n));
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 16.0),
            "the vocoded output stays finite and bounded"
        );
    }

    #[test]
    fn a_dry_mix_passes_the_carrier_through() {
        // mix = 0 is the untouched carrier.
        let car = noise(4_096);
        let out = run(&mut Vocoder::new(15.0, 0.0), &car, &tone(1_000.0, 4_096));
        for (a, b) in car.iter().zip(&out) {
            assert!((a - b).abs() < 1e-6, "dry mix is the carrier verbatim");
        }
    }

    #[test]
    fn set_params_updates_in_place_and_clamps() {
        let mut v = Vocoder::new(15.0, 1.0);
        assert!(v.set_params(&[40.0, 0.5]));
        assert!((v.release_ms - 40.0).abs() < 1e-6);
        assert!((v.mix - 0.5).abs() < 1e-6);
        assert!(v.set_params(&[9_999.0, 9.0]));
        assert!((v.release_ms - 500.0).abs() < 1e-6);
        assert!((v.mix - 1.0).abs() < 1e-6);
    }
}
