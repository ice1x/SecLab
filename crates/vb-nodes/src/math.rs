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

/// Subtracts the second input from the first sample-wise: `out = a - b`.
///
/// The difference counterpart to [`Add`] — useful for phase cancellation, mid/side tricks,
/// or forming an error/difference control signal.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sub;

impl Processor for Sub {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, (a, b)) in outputs[0]
            .iter_mut()
            .zip(inputs[0].iter().zip(inputs[1].iter()))
            .take(ctx.frames)
        {
            *o = *a - *b;
        }
    }
}

/// Full-wave rectifier: `out = |in|`.
///
/// Folds the negative half of a signal up, doubling its frequency for audio (a cheap
/// octave-up fuzz) and turning a bipolar control signal into a unipolar one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rectify;

impl Processor for Rectify {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, x) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
            *o = x.abs();
        }
    }
}

/// Attenuverter: scale (with optional inversion) then offset a signal: `out = in*scale + offset`.
///
/// The workhorse CV utility. A negative `scale` inverts; `offset` shifts the signal's centre.
/// It subsumes a plain invert (`scale = -1, offset = 0`) and a DC offset (`scale = 1`), and is
/// what maps a bipolar modulator into the exact range a destination wants.
#[derive(Debug, Clone, Copy)]
pub struct Attenuvert {
    /// Multiplier applied before the offset (negative inverts).
    pub scale: f32,
    /// Constant added after scaling.
    pub offset: f32,
}

impl Attenuvert {
    /// Creates an attenuverter with `scale` and `offset`.
    #[must_use]
    pub fn new(scale: f32, offset: f32) -> Self {
        Self { scale, offset }
    }
}

impl Processor for Attenuvert {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, x) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
            *o = *x * self.scale + self.offset;
        }
    }

    /// Live-updates `scale`, `offset` (registry order). Stateless, so the update is seamless.
    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&v) = values.first() {
            self.scale = v;
        }
        if let Some(&v) = values.get(1) {
            self.offset = v;
        }
        true
    }
}

/// Sample-wise minimum of two inputs: `out = min(a, b)`.
///
/// A signal utility and a logic gate in one: on bipolar audio it takes the lower of two
/// signals; on `0`/`1` gates it is a logical AND (high only when both are high). Pairs with
/// [`Max`] to combine the outputs of comparators, clocks or envelopes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Min;

impl Processor for Min {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, (a, b)) in outputs[0]
            .iter_mut()
            .zip(inputs[0].iter().zip(inputs[1].iter()))
            .take(ctx.frames)
        {
            *o = a.min(*b);
        }
    }
}

/// Sample-wise maximum of two inputs: `out = max(a, b)`.
///
/// The counterpart to [`Min`]: the higher of two signals, or a logical OR on `0`/`1` gates
/// (high when either is high). Useful for merging triggers or taking an envelope's peak.
#[derive(Debug, Clone, Copy, Default)]
pub struct Max;

impl Processor for Max {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (o, (a, b)) in outputs[0]
            .iter_mut()
            .zip(inputs[0].iter().zip(inputs[1].iter()))
            .take(ctx.frames)
        {
            *o = a.max(*b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Add, Attenuvert, Clamp, Gain, Max, Min, Mix, Mul, Rectify, Sub};
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
    fn min_takes_the_lower_input() {
        assert_eq!(
            run2(&mut Min, &[1.0, 5.0, -2.0], &[3.0, 2.0, -1.0]),
            vec![1.0, 2.0, -2.0]
        );
    }

    #[test]
    fn max_takes_the_higher_input() {
        assert_eq!(
            run2(&mut Max, &[1.0, 5.0, -2.0], &[3.0, 2.0, -1.0]),
            vec![3.0, 5.0, -1.0]
        );
    }

    #[test]
    fn min_and_max_act_as_gate_logic() {
        // On 0/1 gates: min = AND, max = OR.
        let a = [0.0, 0.0, 1.0, 1.0];
        let b = [0.0, 1.0, 0.0, 1.0];
        assert_eq!(run2(&mut Min, &a, &b), vec![0.0, 0.0, 0.0, 1.0], "AND");
        assert_eq!(run2(&mut Max, &a, &b), vec![0.0, 1.0, 1.0, 1.0], "OR");
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

    #[test]
    fn sub_differences_inputs() {
        assert_eq!(run2(&mut Sub, &[5.0, 2.0], &[3.0, 8.0]), vec![2.0, -6.0]);
    }

    #[test]
    fn rectify_takes_absolute_value() {
        assert_eq!(
            run1(&mut Rectify, &[-2.0, 0.0, 3.0, -0.5]),
            vec![2.0, 0.0, 3.0, 0.5]
        );
    }

    #[test]
    fn attenuvert_scales_and_offsets() {
        // out = in*scale + offset.
        assert_eq!(
            run1(&mut Attenuvert::new(2.0, 1.0), &[0.0, 1.0, -1.0]),
            vec![1.0, 3.0, -1.0]
        );
        // A negative scale inverts (subsumes a plain invert).
        assert_eq!(
            run1(&mut Attenuvert::new(-1.0, 0.0), &[0.5, -0.25]),
            vec![-0.5, 0.25]
        );
    }
}
