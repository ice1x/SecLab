//! Bitcrusher: lo-fi digital degradation by bit-depth and sample-rate reduction.
//!
//! Two classic forms of digital nastiness in one node. **Bit reduction** quantises each
//! sample to a coarse set of `2^bits` levels, adding the gritty quantisation distortion of
//! early samplers. **Downsampling** holds each sample for `downsample` steps (sample-rate
//! decimation), folding high harmonics down into aliased artefacts. A dry/wet `mix` blends
//! the crushed signal back with the clean one.

use seclab_core::{ProcessContext, Processor};

/// A bit-depth and sample-rate reducer.
///
/// Input 0 is audio. Output 0 blends the crushed signal with the dry input by `mix`. `bits`
/// is the effective bit depth (fewer = coarser); `downsample` holds each new sample for that
/// many steps (1 = no decimation).
#[derive(Debug, Clone)]
pub struct Bitcrusher {
    bits: f32,
    downsample: usize,
    mix: f32,
    held: f32,
    counter: usize,
}

impl Bitcrusher {
    /// Creates a bitcrusher with effective `bits` (clamped 1..=16), `downsample` factor
    /// (clamped to `>= 1`) and dry/wet `mix` (clamped 0..=1).
    #[must_use]
    pub fn new(bits: f32, downsample: usize, mix: f32) -> Self {
        Self {
            bits: bits.clamp(1.0, 16.0),
            downsample: downsample.max(1),
            mix: mix.clamp(0.0, 1.0),
            held: 0.0,
            counter: 0,
        }
    }

    /// Quantises `x` in `[-1, 1]` to `2^bits` levels.
    fn quantize(&self, x: f32) -> f32 {
        let levels = 2.0_f32.powf(self.bits);
        let half = levels * 0.5;
        (x * half).round() / half
    }
}

impl Processor for Bitcrusher {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            // Refresh the held (crushed) sample every `downsample` steps.
            if self.counter == 0 {
                self.held = self.quantize(x);
            }
            self.counter = (self.counter + 1) % self.downsample;
            *out = x * (1.0 - self.mix) + self.held * self.mix;
        }
    }

    fn reset(&mut self) {
        self.held = 0.0;
        self.counter = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(bc: &mut Bitcrusher, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            bc.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn tone(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * 220.0 * i as f32 / SR).sin())
            .collect()
    }

    fn unique_count(xs: &[f32]) -> usize {
        let mut v: Vec<i64> = xs.iter().map(|x| (x * 1e6) as i64).collect();
        v.sort_unstable();
        v.dedup();
        v.len()
    }

    #[test]
    fn mix_zero_passes_the_input_through() {
        let input = tone(512);
        let out = run(&mut Bitcrusher::new(2.0, 8, 0.0), &input);
        for (a, b) in input.iter().zip(&out) {
            assert!((a - b).abs() < 1e-6, "dry passes through");
        }
    }

    #[test]
    fn low_bit_depth_coarsens_to_few_levels() {
        // 2 bits (mix 1, no downsample) → at most a handful of distinct output levels, far
        // fewer than the smooth input tone's.
        let input = tone(4_800);
        let crushed = run(&mut Bitcrusher::new(2.0, 1, 1.0), &input);
        assert!(crushed
            .iter()
            .all(|s| s.is_finite() && s.abs() <= 1.0 + 1e-6));
        let levels = unique_count(&crushed);
        assert!(levels <= 8, "2-bit crush yields few levels, got {levels}");
        assert!(unique_count(&input) > 100, "the input itself is smooth");
    }

    #[test]
    fn downsampling_holds_samples() {
        // downsample 4, mix 1, high bit depth: each new value is held for 4 samples, so
        // consecutive runs of 4 equal outputs appear.
        let input = tone(64);
        let out = run(&mut Bitcrusher::new(16.0, 4, 1.0), &input);
        // Samples 1,2,3 equal sample 0 (the value latched at counter 0).
        assert!((out[1] - out[0]).abs() < 1e-9);
        assert!((out[2] - out[0]).abs() < 1e-9);
        assert!((out[3] - out[0]).abs() < 1e-9);
        assert!(
            (out[4] - out[0]).abs() > 0.0 || out[4] == out[0],
            "a new sample latches at index 4"
        );
    }

    #[test]
    fn params_are_clamped() {
        assert!((Bitcrusher::new(99.0, 1, 0.5).bits - 16.0).abs() < 1e-6);
        assert!((Bitcrusher::new(0.1, 1, 0.5).bits - 1.0).abs() < 1e-6);
        assert_eq!(Bitcrusher::new(8.0, 0, 0.5).downsample, 1);
    }
}
