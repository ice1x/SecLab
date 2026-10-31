//! A human-facing Markdown node reference, auto-generated from the registry.
//!
//! [`crate::node_reference`] (000115) renders a token-frugal card corpus for the LLM;
//! this is the readable manual for people — every node grouped by category, with its
//! ports and parameters spelled out. It is generated straight from the registry so it
//! cannot drift from the code: `docs/nodes.md` is committed from this function and a test
//! fails if the two diverge (regenerate with `cargo xtask` / the snippet in that test).

use seclab_core::{Parameter, Port, SignalType};

use crate::registry::{NodeDescriptor, Registry};

/// Signal-type tag matching the prompt/reference vocabulary.
fn type_tag(signal: SignalType) -> &'static str {
    match signal {
        SignalType::Audio => "audio",
        SignalType::Control => "control",
        SignalType::Event => "event",
    }
}

/// Shortest round-tripping `f32` (`440.0` → `440`, `0.707` → `0.707`).
fn num(v: f32) -> String {
    format!("{v}")
}

/// Renders a port list as an inline Markdown fragment, or `_(none)_`.
fn ports_md(ports: &[Port]) -> String {
    if ports.is_empty() {
        return "_(none)_".to_owned();
    }
    ports
        .iter()
        .map(|p| format!("`{}` ({})", p.name, type_tag(p.signal)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Appends the parameter lines for a node (each a `- ` bullet), or nothing.
fn params_md(out: &mut String, params: &[Parameter]) {
    if params.is_empty() {
        out.push_str("- **Parameters:** _(none)_\n");
        return;
    }
    out.push_str("- **Parameters:**\n");
    for p in params {
        out.push_str(&format!(
            "  - `{}` — range {}..{}, default {}\n",
            p.name,
            num(p.min),
            num(p.max),
            num(p.default)
        ));
    }
}

/// The full Markdown node reference: nodes grouped by category (sorted), each with its
/// ports and parameters. Deterministic, so a committed copy can be diff-checked.
#[must_use]
pub fn node_reference_markdown(registry: &Registry) -> String {
    let mut descriptors: Vec<&NodeDescriptor> =
        registry.kinds().filter_map(|k| registry.get(k)).collect();
    descriptors.sort_by(|a, b| {
        a.category()
            .cmp(b.category())
            .then_with(|| a.kind().cmp(b.kind()))
    });

    let mut out = String::from(
        "# SecLab node reference\n\n\
         Auto-generated from the registry — do not edit by hand.\n",
    );

    let mut current_category = "";
    for descriptor in descriptors {
        if descriptor.category() != current_category {
            current_category = descriptor.category();
            out.push_str(&format!("\n## {current_category}\n"));
        }
        let node = descriptor.graph_node();
        out.push_str(&format!("\n### {}\n\n", descriptor.kind()));
        out.push_str(&format!("- **Inputs:** {}\n", ports_md(&node.inputs)));
        out.push_str(&format!("- **Outputs:** {}\n", ports_md(&node.outputs)));
        params_md(&mut out, &descriptor.params());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_covers_every_kind_grouped_by_category() {
        let registry = Registry::with_builtins();
        let md = node_reference_markdown(&registry);
        assert!(md.starts_with("# SecLab node reference"));
        for kind in registry.kinds() {
            assert!(
                md.contains(&format!("### {kind}\n")),
                "missing section for {kind}"
            );
        }
        // Category headers and a known node's details are present.
        assert!(md.contains("## generator"));
        assert!(md.contains("### oscillator"));
        assert!(md.contains("`waveform` — range 0..3, default 0"));
        assert!(
            md.contains("- **Inputs:** _(none)_"),
            "oscillator has no inputs"
        );
    }

    /// Guards the committed `docs/nodes.md` against drift from the registry. If this
    /// fails, regenerate the file — e.g.:
    /// `std::fs::write("docs/nodes.md", seclab_nodes::node_reference_markdown(&Registry::with_builtins()))`.
    #[test]
    fn committed_docs_are_in_sync_with_the_registry() {
        let committed = include_str!("../../../docs/nodes.md");
        let generated = node_reference_markdown(&Registry::with_builtins());
        assert_eq!(
            committed, generated,
            "docs/nodes.md is stale — regenerate it from the registry"
        );
    }
}
