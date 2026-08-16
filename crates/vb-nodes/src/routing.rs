//! Routing nodes: splitter (one to many), merger (many to one), selector (pick one).
//!
//! The graph already allows fan-out from any output port, but explicit routing nodes
//! make patches readable and give the selector a switchable path. Unconnected inputs
//! arrive as silent buffers from the engine, so the merger simply sums whatever is
//! wired.

use vb_core::{ProcessContext, Processor};

/// Copies one input to every output.
pub struct Splitter;

impl Processor for Splitter {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(input) = inputs.first() else {
            return;
        };
        for out in outputs.iter_mut() {
            out[..ctx.frames].copy_from_slice(&input[..ctx.frames]);
        }
    }
}

/// Sums every input into one output.
pub struct Merger;

impl Processor for Merger {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(out) = outputs.first_mut() else {
            return;
        };
        out[..ctx.frames].fill(0.0);
        for input in inputs {
            for (o, x) in out.iter_mut().zip(input.iter()).take(ctx.frames) {
                *o += *x;
            }
        }
    }
}

/// Passes exactly one input through, chosen by index.
pub struct Selector {
    index: usize,
}

impl Selector {
    /// Creates a selector initially passing input `index`.
    #[must_use]
    pub fn new(index: usize) -> Self {
        Self { index }
    }
}

impl Processor for Selector {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(out) = outputs.first_mut() else {
            return;
        };
        // A selector pointing past the wired inputs plays silence, not a panic.
        match inputs.get(self.index.min(inputs.len().saturating_sub(1))) {
            Some(input) if !inputs.is_empty() => {
                out[..ctx.frames].copy_from_slice(&input[..ctx.frames]);
            }
            _ => out[..ctx.frames].fill(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: ProcessContext = ProcessContext {
        sample_rate: 48_000.0,
        frames: 4,
    };

    #[test]
    fn splitter_copies_the_input_to_every_output() {
        let input = [1.0, -2.0, 3.0, -4.0_f32];
        let (mut a, mut b) = ([0.0_f32; 4], [0.0_f32; 4]);
        Splitter.process(&CTX, &[&input], &mut [&mut a, &mut b]);
        assert_eq!(a, input);
        assert_eq!(b, input);
    }

    #[test]
    fn merger_sums_all_inputs() {
        let (a, b, c) = ([1.0_f32; 4], [10.0_f32; 4], [-0.5_f32; 4]);
        let mut out = [9.9_f32; 4];
        Merger.process(&CTX, &[&a, &b, &c], &mut [&mut out]);
        assert_eq!(out, [10.5; 4]);

        // No inputs at all: clean silence, not stale buffer contents.
        let mut empty = [7.0_f32; 4];
        Merger.process(&CTX, &[], &mut [&mut empty]);
        assert_eq!(empty, [0.0; 4]);
    }

    #[test]
    fn selector_picks_the_indexed_input() {
        let (a, b) = ([1.0_f32; 4], [2.0_f32; 4]);
        let mut out = [0.0_f32; 4];
        Selector::new(0).process(&CTX, &[&a, &b], &mut [&mut out]);
        assert_eq!(out, [1.0; 4]);
        Selector::new(1).process(&CTX, &[&a, &b], &mut [&mut out]);
        assert_eq!(out, [2.0; 4]);
    }

    #[test]
    fn selector_out_of_range_clamps_and_no_inputs_is_silence() {
        let (a, b) = ([1.0_f32; 4], [2.0_f32; 4]);
        let mut out = [0.0_f32; 4];
        Selector::new(99).process(&CTX, &[&a, &b], &mut [&mut out]);
        assert_eq!(out, [2.0; 4], "clamped to the last wired input");

        let mut silent = [5.0_f32; 4];
        Selector::new(0).process(&CTX, &[], &mut [&mut silent]);
        assert_eq!(silent, [0.0; 4]);
    }
}
