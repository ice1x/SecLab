//! Built-in **factory instrument patches** the user can recall from the editor (000165).
//!
//! Each entry is a recognisable instrument clone, its `.vbp` RON compiled in from `examples/`
//! (the same committed files the plugin ships and the tests validate). This is a pure lookup
//! table — no egui — so the list and the name→RON resolution are unit-testable; the palette's
//! "factory patches" section renders it and loading goes through
//! [`crate::EditorApp::load_factory_patch`].

/// One factory patch: a display name and its committed RON.
pub struct FactoryPatch {
    /// Human-readable instrument name shown in the palette.
    pub name: &'static str,
    /// The committed `.vbp` RON for the patch.
    pub ron: &'static str,
}

/// Every factory instrument, in display order.
pub const FACTORY_PATCHES: &[FactoryPatch] = &[
    FactoryPatch {
        name: "Moog Minitaur",
        ron: include_str!("../../../examples/minitaur.ron"),
    },
    FactoryPatch {
        name: "Minimoog Model D",
        ron: include_str!("../../../examples/minimoog.ron"),
    },
    FactoryPatch {
        name: "Moog Grandmother",
        ron: include_str!("../../../examples/grandmother.ron"),
    },
    FactoryPatch {
        name: "Behringer Neutron",
        ron: include_str!("../../../examples/neutron.ron"),
    },
    FactoryPatch {
        name: "Behringer Edge",
        ron: include_str!("../../../examples/edge.ron"),
    },
    FactoryPatch {
        name: "Korg Minilogue XD",
        ron: include_str!("../../../examples/minilogue.ron"),
    },
    FactoryPatch {
        name: "Roland TB-303",
        ron: include_str!("../../../examples/tb303.ron"),
    },
    FactoryPatch {
        name: "Roland SH-101",
        ron: include_str!("../../../examples/sh101.ron"),
    },
    FactoryPatch {
        name: "Roland Juno-60",
        ron: include_str!("../../../examples/juno60.ron"),
    },
    FactoryPatch {
        name: "Wavetable Pad",
        ron: include_str!("../../../examples/wavetable_pad.ron"),
    },
    FactoryPatch {
        name: "Plucked String",
        ron: include_str!("../../../examples/pluck_string.ron"),
    },
    FactoryPatch {
        name: "Sync Lead",
        ron: include_str!("../../../examples/sync_lead.ron"),
    },
    FactoryPatch {
        name: "Vowel Pad",
        ron: include_str!("../../../examples/vowel_pad.ron"),
    },
    FactoryPatch {
        name: "Granular Cloud",
        ron: include_str!("../../../examples/granular_cloud.ron"),
    },
    FactoryPatch {
        name: "West-Coast Fold",
        ron: include_str!("../../../examples/west_coast_fold.ron"),
    },
    FactoryPatch {
        name: "Ring Bell",
        ron: include_str!("../../../examples/ring_bell.ron"),
    },
    FactoryPatch {
        name: "Struck Resonator",
        ron: include_str!("../../../examples/resonator_pluck.ron"),
    },
    FactoryPatch {
        name: "Lo-Fi Crush",
        ron: include_str!("../../../examples/lofi_crush.ron"),
    },
    FactoryPatch {
        name: "Random Steps",
        ron: include_str!("../../../examples/random_steps.ron"),
    },
    FactoryPatch {
        name: "Flanged Sweep",
        ron: include_str!("../../../examples/flanged_sweep.ron"),
    },
    FactoryPatch {
        name: "Metallic Shift",
        ron: include_str!("../../../examples/metallic_shift.ron"),
    },
    FactoryPatch {
        name: "Octave Stack",
        ron: include_str!("../../../examples/octave_stack.ron"),
    },
    FactoryPatch {
        name: "Spectral Bloom",
        ron: include_str!("../../../examples/spectral_bloom.ron"),
    },
    FactoryPatch {
        name: "Vocoder Choir",
        ron: include_str!("../../../examples/vocoder_choir.ron"),
    },
    FactoryPatch {
        name: "Drum kit",
        ron: include_str!("../../../examples/drumkit.ron"),
    },
];

/// Looks up a factory patch by its exact display name.
#[must_use]
pub fn factory_patch(name: &str) -> Option<&'static FactoryPatch> {
    FACTORY_PATCHES.iter().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_factory_patch_parses_and_is_valid() {
        // Anti-rot: a renamed node or a broken example would fail here (as it does in the
        // plugin's examples test), so the palette never offers a patch that won't load.
        for p in FACTORY_PATCHES {
            let doc = vb_project::from_ron(p.ron)
                .unwrap_or_else(|e| panic!("factory patch '{}' fails to parse: {e}", p.name));
            assert!(doc.is_valid(), "factory patch '{}' is not valid", p.name);
        }
    }

    #[test]
    fn factory_patch_resolves_by_name_and_rejects_unknowns() {
        assert!(factory_patch("Moog Minitaur").is_some());
        assert!(factory_patch("Behringer Edge").is_some());
        assert!(factory_patch("No Such Synth").is_none());
    }
}
