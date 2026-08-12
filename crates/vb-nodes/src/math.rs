//! Stateless math / utility nodes: gain, add, multiply, crossfade mix and clamp.

use vb_core::{ProcessContext, Processor};

/// Scales its single input by a constant: `out = in * gain`.
#[derive(Debug, Clone, Copy)]
pub struct Gain {
    /// Linear gain factor.
    pub gain: f32,
}

impl Gain {
    /// Creates a gain node.
    #[must_use]
    pub fn new(gain: f32) -> Self {
        Self { gain }
    }
}

impl Processor for Gain {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, x) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
            *o = *x * self.gain;
        }
    }
}

/// Adds two inputs sample-wise: `out = a + b`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Add;

impl Processor for Add {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, (a, b)) in outputs[0]
            .iter_mut()
            .zip(inputs[0].iter().zip(inputs[1].iter()))
            .take(ctx.frames)
        {
            *o = *a + *b;
        }
    }
}

/// Multiplies two inputs sample-wise (ring-mod / amplitude): `out = a * b`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mul;

impl Processor for Mul {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, (a, b)) in outputs[0]
            .iter_mut()
            .zip(inputs[0].iter().zip(inputs[1].iter()))
            .take(ctx.frames)
        {
            *o = *a * *b;
        }
    }
}

/// Linear crossfade between two inputs: `out = a*(1-t) + b*t`, `t` clamped to `[0, 1]`.
#[derive(Debug, Clone, Copy)]
pub struct Mix {
    /// Blend position: 0 = all `a`, 1 = all `b`.
    pub t: f32,
}

impl Mix {
    /// Creates a mix node with blend position `t`.
    #[must_use]
    pub fn new(t: f32) -> Self {
        Self { t }
    }
}

impl Processor for Mix {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let t = self.t.clamp(0.0, 1.0);
        for (o, (a, b)) in outputs[0]
            .iter_mut()
            .zip(inputs[0].iter().zip(inputs[1].iter()))
            .take(ctx.frames)
        {
            *o = *a * (1.0 - t) + *b * t;
        }
    }
}

/// Clamps its single input to `[min, max]`.
#[derive(Debug, Clone, Copy)]
pub struct Clamp {
    /// Lower bound.
    pub min: f32,
    /// Upper bound.
    pub max: f32,
}

impl Clamp {
    /// Creates a clamp node.
    #[must_use]
    pub fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }
}

impl Processor for Clamp {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, x) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
            *o = x.clamp(self.min, self.max);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Add, Clamp, Gain, Mix, Mul};
    use vb_core::{ProcessContext, Processor};

    fn ctx(frames: usize) -> ProcessContext {
        ProcessContext {
            sample_rate: 48_000.0,
            frames,
        }
    }

    fn run1(p: &mut dyn Processor, input: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0_f32; input.len()];
        let ins: [&[f32]; 1] = [input];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        p.process(&ctx(input.len()), &ins, &mut outs);
        out
    }

    fn run2(p: &mut dyn Processor, a: &[f32], b: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0_f32; a.len()];
        let ins: [&[f32]; 2] = [a, b];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        p.process(&ctx(a.len()), &ins, &mut outs);
        out
    }

    #[test]
    fn gain_scales() {
        assert_eq!(
            run1(&mut Gain::new(2.0), &[1.0, -2.0, 3.0]),
            vec![2.0, -4.0, 6.0]
        );
    }

    #[test]
    fn add_sums_inputs() {
        assert_eq!(run2(&mut Add, &[1.0, 2.0], &[10.0, 20.0]), vec![11.0, 22.0]);
    }

    #[test]
    fn mul_multiplies_inputs() {
        assert_eq!(run2(&mut Mul, &[2.0, 3.0], &[4.0, 5.0]), vec![8.0, 15.0]);
    }

    #[test]
    fn mix_crossfades_and_clamps_t() {
        assert_eq!(run2(&mut Mix::new(0.0), &[1.0], &[9.0]), vec![1.0]);
        assert_eq!(run2(&mut Mix::new(1.0), &[1.0], &[9.0]), vec![9.0]);
        assert_eq!(run2(&mut Mix::new(0.5), &[2.0], &[4.0]), vec![3.0]);
        // t is clamped, so 2.0 behaves like 1.0.
        assert_eq!(run2(&mut Mix::new(2.0), &[1.0], &[9.0]), vec![9.0]);
    }

    #[test]
    fn clamp_bounds_signal() {
        assert_eq!(
            run1(&mut Clamp::new(-1.0, 1.0), &[-2.0, 0.5, 3.0]),
            vec![-1.0, 0.5, 1.0]
        );
    }
}
