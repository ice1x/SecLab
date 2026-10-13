//! A small in-place radix-2 Cooley–Tukey FFT, used by the spectral nodes.
//!
//! The project keeps no FFT dependency (the Hilbert-based `freq-shifter` hand-rolls its DSP
//! too), so this is a self-contained, allocation-free transform: the caller owns the buffer and
//! the transform only shuffles and combines it in place.
//!
//! The buffer is interleaved complex — `[re0, im0, re1, im1, …]` — of length `2·n`, where `n`
//! is the number of complex points and **must be a power of two**. `sign = -1.0` is the forward
//! transform `X[k] = Σ x[n]·e^(−2πi·kn/N)`; `sign = +1.0` is the inverse with the **same**
//! (unnormalised) convention, so `inverse(forward(x)) = N·x`. This matches the classic
//! `smbFft` convention the phase vocoder is written against, so its magnitude/phase scaling
//! carries over unchanged.

/// In-place radix-2 FFT of `n` complex points held interleaved in `buf` (`buf.len() == 2·n`).
///
/// `sign` is `-1.0` for the forward transform and `+1.0` for the (unnormalised) inverse.
///
/// # Panics
/// Panics if `n` is not a power of two or `buf.len() != 2 * n`.
pub fn fft(buf: &mut [f32], n: usize, sign: f32) {
    assert!(n.is_power_of_two(), "fft size must be a power of two");
    assert_eq!(buf.len(), 2 * n, "buffer must hold 2·n interleaved floats");
    if n < 2 {
        return;
    }

    // Decimation-in-time bit-reversal permutation (Gold–Rader).
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            buf.swap(2 * i, 2 * j);
            buf.swap(2 * i + 1, 2 * j + 1);
        }
    }

    // Danielson–Lanczos butterflies, stage length 2, 4, …, n.
    let mut len = 2usize;
    while len <= n {
        let ang = sign * core::f32::consts::TAU / len as f32;
        let (wstep_r, wstep_i) = (ang.cos(), ang.sin());
        let half = len / 2;
        let mut base = 0usize;
        while base < n {
            let (mut wr, mut wi) = (1.0_f32, 0.0_f32);
            for k in 0..half {
                let i1 = 2 * (base + k);
                let i2 = 2 * (base + k + half);
                let (xr, xi) = (buf[i2], buf[i2 + 1]);
                let tr = xr * wr - xi * wi;
                let ti = xr * wi + xi * wr;
                buf[i2] = buf[i1] - tr;
                buf[i2 + 1] = buf[i1 + 1] - ti;
                buf[i1] += tr;
                buf[i1 + 1] += ti;
                let nwr = wr * wstep_r - wi * wstep_i;
                wi = wr * wstep_i + wi * wstep_r;
                wr = nwr;
            }
            base += len;
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::TAU;

    /// Naive O(n²) DFT reference: returns interleaved `[re, im]` for the forward transform.
    fn naive_dft(input: &[f32]) -> Vec<f32> {
        let n = input.len() / 2;
        let mut out = vec![0.0_f32; 2 * n];
        for (k, chunk) in out.chunks_mut(2).enumerate() {
            let (mut re, mut im) = (0.0_f32, 0.0_f32);
            for m in 0..n {
                let ang = -TAU * (k as f32) * (m as f32) / n as f32;
                let (xr, xi) = (input[2 * m], input[2 * m + 1]);
                let (s, c) = ang.sin_cos();
                // e^(ang·i) = (cos, sin); (xr+ixi)(c+is) = (xr·c − xi·s) + i(xr·s + xi·c)
                re += xr * c - xi * s;
                im += xr * s + xi * c;
            }
            chunk[0] = re;
            chunk[1] = im;
        }
        out
    }

    fn interleave_real(samples: &[f32]) -> Vec<f32> {
        let mut v = vec![0.0_f32; 2 * samples.len()];
        for (i, &x) in samples.iter().enumerate() {
            v[2 * i] = x;
        }
        v
    }

    #[test]
    fn forward_matches_the_naive_dft() {
        let n = 64;
        let samples: Vec<f32> = (0..n)
            .map(|i| (TAU * 3.0 * i as f32 / n as f32).sin() + 0.5 * (i as f32 * 0.1).cos())
            .collect();
        let mut buf = interleave_real(&samples);
        let reference = naive_dft(&buf);
        fft(&mut buf, n, -1.0);
        for (a, b) in buf.iter().zip(&reference) {
            assert!((a - b).abs() < 1e-2, "fft bin {a} vs dft {b}");
        }
    }

    #[test]
    fn a_pure_tone_lands_in_one_bin_pair() {
        // A real cosine at bin 5 shows up as energy at bins 5 and n−5 (its mirror), nowhere else.
        let n = 64;
        let samples: Vec<f32> = (0..n)
            .map(|i| (TAU * 5.0 * i as f32 / n as f32).cos())
            .collect();
        let mut buf = interleave_real(&samples);
        fft(&mut buf, n, -1.0);
        let mag = |k: usize| (buf[2 * k].powi(2) + buf[2 * k + 1].powi(2)).sqrt();
        assert!(mag(5) > 0.4 * n as f32, "peak at bin 5");
        assert!(mag(n - 5) > 0.4 * n as f32, "mirror peak at bin n−5");
        for k in 0..n {
            if k != 5 && k != n - 5 {
                assert!(mag(k) < 1e-2, "bin {k} is empty, got {}", mag(k));
            }
        }
    }

    #[test]
    fn inverse_of_forward_is_the_original_times_n() {
        let n = 128;
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                (TAU * 7.0 * i as f32 / n as f32).sin()
                    - 0.3 * (TAU * 11.0 * i as f32 / n as f32).cos()
            })
            .collect();
        let original = interleave_real(&samples);
        let mut buf = original.clone();
        fft(&mut buf, n, -1.0);
        fft(&mut buf, n, 1.0);
        for (rt, orig) in buf.iter().zip(&original) {
            assert!(
                (rt / n as f32 - orig).abs() < 1e-3,
                "ifft/n {} vs original {orig}",
                rt / n as f32
            );
        }
    }

    #[test]
    fn dc_bin_is_the_sum_of_samples() {
        let n = 32;
        let samples: Vec<f32> = (0..n).map(|i| 0.1 + 0.01 * i as f32).collect();
        let mut buf = interleave_real(&samples);
        let sum: f32 = samples.iter().sum();
        fft(&mut buf, n, -1.0);
        assert!((buf[0] - sum).abs() < 1e-3, "DC bin = Σx");
        assert!(buf[1].abs() < 1e-3, "DC imaginary part is zero");
    }
}
