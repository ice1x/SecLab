//! `vb-core` — the structural DSP graph model for VST_bomb.
//!
//! This crate holds the data model that the whole editor and runtime build on:
//! typed [`Port`]s, [`GraphNode`]s and the [`Graph`] container that wires them
//! together while enforcing the connection rules. The [`Processor`] trait is the
//! behavioural extension point DSP nodes implement; cycle detection and scheduling
//! arrive in later epics. Keeping structure and behaviour separate keeps the model
//! easy to serialize, diff and reason about (SOLID).

pub mod graph;
pub mod node;
pub mod port;
pub mod process;
pub mod signal;
pub mod topology;

pub use graph::{Edge, Graph, GraphError};
pub use node::{GraphNode, NodeId};
pub use port::{Port, PortDirection, PortId};
pub use process::{ProcessContext, Processor};
pub use signal::SignalType;
