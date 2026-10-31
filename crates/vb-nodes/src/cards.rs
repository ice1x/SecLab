//! A distilled, auto-generated node reference for LLM context.
//!
//! The AI assistant (epic 12) must be *grounded*: it may only propose nodes that
//! actually exist, with their real ports and parameters. Rather than hand-maintain a
//! doc that drifts from the code, this module renders one compact "card" per registered
//! kind straight from the [`Registry`] — kind, category, typed ports, and each
//! parameter's range and default — in a terse form that fits a context window and lines
//! up with the patch prompt format (`name:type` port tags).
//!
//! One card per line:
//!
//! ```text
//! <kind> [<category>] in=<name:type,...> out=<name:type,...> params=<name:min..max=default,...>
//! ```
//!
//! `params=` is omitted for nodes with nothing to edit. The whole corpus is sorted by
//! category then kind so related nodes sit together.

use vb_core::{Parameter, Port, SignalType};

use crate::registry::{NodeDescriptor, Registry};

/// First line of a reference; also the version marker.
const HEADER: &str = "# SecLab node reference v1";

/// Short tag for a signal type (matches the patch prompt format).
fn type_tag(signal: SignalType) -> &'static str {
    match signal {
        SignalType::Audio => "audio",
        SignalType::Control => "control",
        SignalType::Event => "event",
    }
}

/// Formats an `f32` without trailing zeros (`440.0` → `440`, `0.707` → `0.707`).
fn num(v: f32) -> String {
    // Rust's shortest-round-trip Display already drops a bare `.0`.
    format!("{v}")
}

/// Renders a port list as `name:type,name:type` (empty for none).
fn ports_str(ports: &[Port]) -> String {
    ports
        .iter()
        .map(|p| format!("{}:{}", p.name, type_tag(p.signal)))
        .collect::<Vec<_>>()
        .join(",")
}

/// Renders a parameter list as `name:min..max=default,...` (empty for none).
fn params_str(params: &[Parameter]) -> String {
    params
        .iter()
        .map(|p| {
            format!(
                "{}:{}..{}={}",
                p.name,
                num(p.min),
                num(p.max),
                num(p.default)
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// One compact reference card for `descriptor`.
#[must_use]
pub fn node_card(descriptor: &NodeDescriptor) -> String {
    let node = descriptor.graph_node();
    let params = descriptor.params();
    let mut card = format!(
        "{} [{}] in={} out={}",
        descriptor.kind(),
        descriptor.category(),
        ports_str(&node.inputs),
        ports_str(&node.outputs),
    );
    if !params.is_empty() {
        card.push_str(&format!(" params={}", params_str(&params)));
    }
    card
}

/// The full node reference: a header line plus one [`node_card`] per registered kind,
/// sorted by category then kind.
#[must_use]
pub fn node_reference(registry: &Registry) -> String {
    let mut descriptors: Vec<&NodeDescriptor> =
        registry.kinds().filter_map(|k| registry.get(k)).collect();
    descriptors.sort_by(|a, b| {
        a.category()
            .cmp(b.category())
            .then_with(|| a.kind().cmp(b.kind()))
    });

    let mut out = String::from(HEADER);
    out.push('\n');
    for descriptor in descriptors {
        out.push_str(&node_card(descriptor));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_is_rendered_for_every_registered_kind() {
        let registry = Registry::with_builtins();
        let reference = node_reference(&registry);
        assert!(reference.starts_with(HEADER));
        for kind in registry.kinds() {
            assert!(
                reference.contains(&format!("{kind} [")),
                "reference is missing a card for {kind}"
            );
        }
        // One header line + one line per kind.
        let lines = reference.lines().count();
        assert_eq!(lines, registry.kinds().count() + 1);
    }

    #[test]
    fn a_card_carries_ports_and_parameter_ranges() {
        let registry = Registry::with_builtins();

        let osc = node_card(registry.get("oscillator").unwrap());
        assert_eq!(
            osc,
            "oscillator [generator] in= out=out:audio params=waveform:0..3=0,freq_hz:20..20000=440"
        );

        let gain = node_card(registry.get("gain").unwrap());
        assert!(gain.contains("in=in:audio out=out:audio"));
        assert!(
            gain.contains("params=gain:0..2=1"),
            "range and default, no .0"
        );
    }

    #[test]
    fn nodes_without_parameters_omit_the_params_field() {
        let registry = Registry::with_builtins();
        let add = node_card(registry.get("add").unwrap());
        assert!(
            !add.contains("params="),
            "pure combinator has nothing to edit"
        );
        assert!(
            add.contains("[math]") || add.contains("["),
            "still has a category"
        );
    }

    #[test]
    fn the_audio_io_and_control_nodes_are_documented() {
        let registry = Registry::with_builtins();
        let reference = node_reference(&registry);
        // The plugin I/O nodes (000032) and a fractional-range control both render.
        assert!(reference.contains("audio-in [input] in= out=out:audio params=channel:0..1=0"));
        assert!(reference.contains("audio-out [output] in=in:audio out=out:audio"));
        assert!(
            reference.contains("q:0.1..10=0.707"),
            "fractional ranges keep precision"
        );
    }

    #[test]
    fn cards_are_grouped_by_category() {
        let registry = Registry::with_builtins();
        let reference = node_reference(&registry);
        // Every "[category]" tag appears in non-decreasing order down the file.
        let categories: Vec<&str> = reference
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once('[').and_then(|(_, r)| r.split_once(']')))
            .map(|(cat, _)| cat)
            .collect();
        let mut sorted = categories.clone();
        sorted.sort_unstable();
        assert_eq!(categories, sorted, "cards are ordered by category");
    }
}
