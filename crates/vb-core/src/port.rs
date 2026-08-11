//! Typed connection points on a node.

use crate::signal::SignalType;

/// Whether a port consumes (input) or produces (output) a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    /// Consumes a signal.
    Input,
    /// Produces a signal.
    Output,
}

/// Index of a port within a node's input or output list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortId(pub usize);

/// A typed connection point on a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    /// Human-readable label shown in the editor.
    pub name: String,
    /// Whether the port is an input or an output.
    pub direction: PortDirection,
    /// The signal type carried by the port.
    pub signal: SignalType,
}

impl Port {
    /// Creates a port with an explicit direction.
    #[must_use]
    pub fn new(name: impl Into<String>, direction: PortDirection, signal: SignalType) -> Self {
        Self {
            name: name.into(),
            direction,
            signal,
        }
    }

    /// Creates an input port.
    #[must_use]
    pub fn input(name: impl Into<String>, signal: SignalType) -> Self {
        Self::new(name, PortDirection::Input, signal)
    }

    /// Creates an output port.
    #[must_use]
    pub fn output(name: impl Into<String>, signal: SignalType) -> Self {
        Self::new(name, PortDirection::Output, signal)
    }
}

#[cfg(test)]
mod tests {
    use super::{Port, PortDirection};
    use crate::signal::SignalType;

    #[test]
    fn input_helper_sets_direction_and_signal() {
        let p = Port::input("freq", SignalType::Control);
        assert_eq!(p.direction, PortDirection::Input);
        assert_eq!(p.signal, SignalType::Control);
        assert_eq!(p.name, "freq");
    }

    #[test]
    fn output_helper_sets_direction() {
        let p = Port::output("out", SignalType::Audio);
        assert_eq!(p.direction, PortDirection::Output);
    }
}
