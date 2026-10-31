# Adding your own node

SecLab's DSP library is open: a node is just a type that implements one trait plus a
small descriptor telling the registry how to build it. This guide walks through adding a
custom node end to end. The complete, compile-tested version of everything below lives in
[`crates/seclab-nodes/tests/custom_node.rs`](../crates/seclab-nodes/tests/custom_node.rs) — the
guide and that test are kept in step, so the code here is known to compile and run. A
companion test,
[`crates/seclab-plugin/tests/custom_node_engine.rs`](../crates/seclab-plugin/tests/custom_node_engine.rs),
proves the same custom node also survives the project-document build path the plugin and
editor use (`ProjectDoc` → `engine_from_doc`), and that an *unregistered* kind is a build
error rather than a silent drop.

Our example is a soft-clip **drive**: `out = tanh(amount * in)` — one audio input, one
audio output, one `amount` parameter.

## 1. Implement `Processor`

A node's DSP is a [`seclab_core::Processor`]. `process` is called once per block with the
input channels and a place to write the outputs; `reset` clears any internal state
between runs (ours is stateless).

```rust
use seclab_core::{ProcessContext, Processor};

struct Drive {
    amount: f32,
}

impl Processor for Drive {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        for (out, &x) in outputs[0].iter_mut().zip(input).take(ctx.frames) {
            *out = (self.amount * x).tanh();
        }
    }

    fn reset(&mut self) {}
}
```

Conventions worth following:

- **Read inputs defensively.** An input may be unconnected; `inputs.first().copied()`
  and a fallback keep the node from panicking when a port has no wire.
- **Honour `ctx.frames`.** Write exactly `ctx.frames` samples; buffers may be larger than
  the current block.
- **Stay real-time-safe.** `process` runs on the audio thread — no allocation, locking, or
  I/O. Do that work up front (in the factory) instead.
- **Control-rate inputs** are read the same way; by convention a node samples a control
  input once at the start of the block (see [`crate::Vca`]/[`crate::Vcf`]/[`crate::Vco`]).

## 2. Describe it

A [`seclab_nodes::NodeDescriptor`] tells the registry how to build the node — its ports, its
default processor, its parameters, and how to build a processor from stored parameter
values. **The factories are plain `fn` pointers**, so use free functions or *non-capturing*
closures (a closure that captures nothing coerces to `fn`).

```rust
use seclab_core::{GraphNode, Parameter, Port, SignalType};
use seclab_nodes::NodeDescriptor;

fn drive_descriptor() -> NodeDescriptor {
    NodeDescriptor::new(
        "drive",   // unique kind name
        "effect",  // UI category
        // ports
        || {
            GraphNode::new(
                "drive",
                vec![Port::input("in", SignalType::Audio)],
                vec![Port::output("out", SignalType::Audio)],
            )
        },
        // default processor
        || Box::new(Drive { amount: 1.0 }),
        // editable parameters (name, min, max, default), in UI order
        || vec![Parameter::new("amount", 0.0, 10.0, 1.0)],
        // processor configured from parameter values (index matches `params()` order)
        |_ctx, values| {
            Box::new(Drive {
                amount: values.first().copied().unwrap_or(1.0),
            })
        },
    )
}
```

The last factory, `make_processor_with`, is the one the editor and plugin use when a patch
carries saved parameter values: `values[i]` lines up with the *i*-th entry from
`make_params`. Fall back to the parameter's default when a value is missing so a stale or
truncated list still builds a working node.

Enum-like choices (a waveform, a filter type) are modelled as *stepped float* parameters
for now — e.g. `Parameter::new("waveform", 0.0, 3.0, 0.0)` — and mapped back to the enum
inside the factory. See the built-ins in `crates/seclab-nodes/src/registry.rs` for examples.

## 3. Register it

Add the descriptor to a [`seclab_nodes::Registry`]. Start from `with_builtins()` to keep every
stock node, or from `new()` for an empty set.

```rust
let mut registry = Registry::with_builtins();
registry.register(drive_descriptor());
```

`drive` is now a first-class kind: it shows up in `registry.kinds()`, the editor can place
it, and the project loader can build it by name.

## 4. Use it

Nodes built from the registry drop straight into a [`seclab_core::Graph`] and run in the
engine:

```rust
use std::collections::HashMap;
use seclab_core::{Engine, Graph, PortId, ProcessContext, Processor};

let mut graph = Graph::new();
let osc = graph.add_node(registry.graph_node("oscillator").unwrap());
let drive = graph.add_node(registry.graph_node("drive").unwrap());
graph.connect(osc, PortId(0), drive, PortId(0)).unwrap();

let ctx = seclab_nodes::BuildContext::default();
let mut procs: HashMap<_, Box<dyn Processor>> = HashMap::new();
procs.insert(osc, registry.processor("oscillator").unwrap());
procs.insert(drive, registry.get("drive").unwrap().processor_with(&ctx, &[8.0]));

let mut engine = Engine::compile(&graph, procs, 128).unwrap();
engine.process(&ProcessContext { sample_rate: 48_000.0, frames: 128 });
```

That is the whole extension surface — implement, describe, register, use.

## Checklist

- [ ] `Processor::process` writes `ctx.frames` finite samples and never allocates.
- [ ] Ports in `make_node` match how `process` reads `inputs` / writes `outputs`.
- [ ] `make_params` order matches how `make_processor_with` reads `values`.
- [ ] Unit tests for the DSP (a known input → expected output) and an engine run.
- [ ] If the node ships as a built-in, add it to `builtins()` and regenerate
      `docs/nodes.md` (there is a sync test).

[`seclab_core::Processor`]: ../crates/seclab-core
[`seclab_nodes::NodeDescriptor`]: ../crates/seclab-nodes/src/registry.rs
[`seclab_nodes::Registry`]: ../crates/seclab-nodes/src/registry.rs
[`crate::Vca`]: ../crates/seclab-nodes/src/vca.rs
[`crate::Vcf`]: ../crates/seclab-nodes/src/vcf.rs
[`crate::Vco`]: ../crates/seclab-nodes/src/vco.rs
