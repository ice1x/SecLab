//! Natural-language explanations of a patch or a single node.
//!
//! The other assistant paths produce a patch; this one produces prose. Given a patch (or
//! a node kind) plus the grounding reference, the model explains — in plain language for a
//! musician — what it does and how the signal flows. There is no output contract to
//! validate: the reply is the explanation, so a single call suffices.

use vb_nodes::{node_card, node_reference, Registry};
use vb_project::ProjectDoc;

use crate::provider::{AiError, AiProvider, ChatRequest, Message, ProviderConfig};

/// System framing for an explanation: grounded, prose-only, musician-facing.
const EXPLAIN_PREAMBLE: &str = "You are the SecLab patch assistant. Explain patches and \
nodes to a musician in plain, concise language: what the thing does, how the audio and \
control signals flow through it, and what each node contributes. Be accurate and grounded \
in the node reference below — do not invent nodes or behaviour. Output prose only; never \
emit code or RON.";

/// Why an explanation could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplainError {
    /// The provider call failed.
    Provider(AiError),
    /// The requested node kind is not in the registry.
    UnknownKind(String),
    /// The model returned an empty explanation.
    Empty,
}

impl std::fmt::Display for ExplainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExplainError::Provider(e) => write!(f, "provider error: {e}"),
            ExplainError::UnknownKind(k) => write!(f, "unknown node kind '{k}'"),
            ExplainError::Empty => write!(f, "the assistant returned no explanation"),
        }
    }
}

impl std::error::Error for ExplainError {}

/// Runs one prose completion and returns non-empty content.
fn ask(
    provider: &dyn AiProvider,
    config: &ProviderConfig,
    system: String,
    user: String,
) -> Result<String, ExplainError> {
    let request = ChatRequest::new(config, system, vec![Message::user(user)]);
    let reply = provider
        .complete(&request)
        .map_err(ExplainError::Provider)?;
    if reply.content.trim().is_empty() {
        return Err(ExplainError::Empty);
    }
    Ok(reply.content)
}

/// Explains what `doc` does, grounded in the full node reference.
///
/// # Errors
/// [`ExplainError::Provider`] on a provider failure, or [`ExplainError::Empty`] if the
/// reply is blank.
pub fn explain_patch(
    provider: &dyn AiProvider,
    registry: &Registry,
    config: &ProviderConfig,
    doc: &ProjectDoc,
) -> Result<String, ExplainError> {
    let system = format!(
        "{EXPLAIN_PREAMBLE}\n\n# Available nodes\n\n{}",
        node_reference(registry)
    );
    let user = format!(
        "Explain this patch in plain language for a musician:\n\n{}",
        doc.to_prompt()
    );
    ask(provider, config, system, user)
}

/// Explains a single node `kind`, using its reference card.
///
/// # Errors
/// [`ExplainError::UnknownKind`] if `kind` is not registered (no provider call is made),
/// otherwise [`ExplainError::Provider`] or [`ExplainError::Empty`].
pub fn explain_node(
    provider: &dyn AiProvider,
    registry: &Registry,
    config: &ProviderConfig,
    kind: &str,
) -> Result<String, ExplainError> {
    let descriptor = registry
        .get(kind)
        .ok_or_else(|| ExplainError::UnknownKind(kind.to_owned()))?;
    let system = EXPLAIN_PREAMBLE.to_owned();
    let user = format!(
        "Explain this node for a musician — what it does and how to use it:\n\n{}",
        node_card(descriptor)
    );
    ask(provider, config, system, user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AiError, ScriptedProvider};
    use vb_core::{Graph, GraphNode, Port, PortId, SignalType};

    fn config() -> ProviderConfig {
        ProviderConfig::default()
    }

    fn osc_gain() -> ProjectDoc {
        let mut g = Graph::new();
        let osc = g.add_node(GraphNode::new(
            "oscillator",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let amp = g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(osc, PortId(0), amp, PortId(0)).unwrap();
        ProjectDoc::from_graph(&g)
    }

    #[test]
    fn explain_patch_grounds_the_prompt_and_returns_prose() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&["A sine oscillator feeding a gain stage."]);

        let text = explain_patch(&provider, &registry, &config(), &osc_gain()).unwrap();
        assert!(text.contains("gain stage"));

        let request = &provider.requests()[0];
        assert!(
            request.system.contains("Explain patches"),
            "explanation framing"
        );
        assert!(
            request.system.contains("oscillator [generator]"),
            "grounded"
        );
        assert!(
            request.messages[0].content.contains("node 0 oscillator"),
            "the patch is described in prompt form"
        );
    }

    #[test]
    fn explain_node_uses_the_cards_card() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&["Biquad is a versatile filter."]);

        let text = explain_node(&provider, &registry, &config(), "biquad").unwrap();
        assert!(text.contains("filter"));
        assert!(provider.requests()[0].messages[0]
            .content
            .contains("biquad [filter]"));
    }

    #[test]
    fn explain_node_rejects_an_unknown_kind_without_calling_the_provider() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&["should not be used"]);
        let err = explain_node(&provider, &registry, &config(), "nope").unwrap_err();
        assert_eq!(err, ExplainError::UnknownKind("nope".to_owned()));
        assert!(provider.requests().is_empty(), "no wasted call");
    }

    #[test]
    fn an_empty_reply_is_an_error() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&["   "]);
        let err = explain_patch(&provider, &registry, &config(), &osc_gain()).unwrap_err();
        assert_eq!(err, ExplainError::Empty);
    }

    #[test]
    fn a_provider_failure_propagates() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::new(vec![Err(AiError::MissingKey)]);
        let err = explain_patch(&provider, &registry, &config(), &osc_gain()).unwrap_err();
        assert_eq!(err, ExplainError::Provider(AiError::MissingKey));
    }
}
