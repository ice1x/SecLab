//! `vb-runtime` — real-time graph execution for SecLab.
//!
//! [`vb_core::Engine`] is the reference evaluator: correct, simple, and free to allocate.
//! That is fine for tests and offline rendering but not for an audio callback, where a
//! single heap allocation can miss a deadline and click.
//!
//! [`RtEngine`] runs the same graph with the same [`Processor`]s, but every buffer and
//! routing table is allocated once in [`RtEngine::compile`]. [`RtEngine::process`] only
//! reads and writes already-sized memory — no allocation, no locks, no `Vec` growth. The
//! per-node slice tables live on the stack, which is why a node may expose at most
//! [`MAX_PORTS_PER_NODE`] ports per side.

pub mod engine;
pub mod swap;

pub use engine::{RtEngine, RtEngineError, MAX_PORTS_PER_NODE};
pub use swap::{engine_swap, EngineReceiver, EngineSender};
