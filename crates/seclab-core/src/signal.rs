//! Signal types carried by ports, and the compatibility rule between them.

/// The kind of signal a port carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SignalType {
    /// Audio-rate sample stream.
    Audio,
    /// Control-rate / CV value.
    Control,
    /// Discrete events (e.g. MIDI).
    Event,
}

impl SignalType {
    /// Whether a signal of this type may feed a port of `other`'s type.
    ///
    /// Compatibility is strict equality for now; any conversion (e.g. audio→control)
    /// must go through an explicit node rather than an implicit cast.
    #[must_use]
    pub fn compatible_with(self, other: SignalType) -> bool {
        self == other
    }
}

#[cfg(test)]
mod tests {
    use super::SignalType;

    #[test]
    fn same_type_is_compatible() {
        assert!(SignalType::Audio.compatible_with(SignalType::Audio));
        assert!(SignalType::Control.compatible_with(SignalType::Control));
        assert!(SignalType::Event.compatible_with(SignalType::Event));
    }

    #[test]
    fn different_types_are_incompatible() {
        assert!(!SignalType::Audio.compatible_with(SignalType::Control));
        assert!(!SignalType::Event.compatible_with(SignalType::Audio));
    }
}
