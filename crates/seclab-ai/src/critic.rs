//! The critic pass: an Occam's-razor review of a patch (and the goal behind it),
//! surfacing redundant nodes or routing and the trade-offs as short bullet points.
//!
//! The other skills build or fix a patch; this one pushes back on it. Grounded in the
//! node reference, the model is asked to argue for the simplest design that still meets
//! the goal — flagging a node that earns its place poorly, a route that could be one hop
//! shorter, or a genuine trade-off worth stating — and to reply as a terse bullet list a
//! UI can show inline. It reads the patch; it does not change it.

use seclab_nodes::{node_reference, Registry};
use seclab_project::ProjectDoc;

use crate::provider::{AiError, AiProvider, ChatRequest, Message, ProviderConfig};

/// System framing for the critic: grounded, Occam-first, bullet-only.
const CRITIC_PREAMBLE: &str = "You are a critical patch reviewer for SecLab. Apply \
Occam's razor: argue for the simplest design that still meets the goal. Flag redundant \
nodes or routing, a node that earns its place poorly, and any real trade-off worth \
stating — but do not invent problems. Be grounded in the node reference below. Reply as \
short bullet points, one per line, each starting with \"- \". If the patch is already \
minimal for its goal, say so in a single bullet.";

/// Why a critique could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CritiqueError {
    /// The provider call failed.
    Provider(AiError),
    /// The model returned nothing usable.
    Empty,
}

impl std::fmt::Display for CritiqueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CritiqueError::Provider(e) => write!(f, "provider error: {e}"),
            CritiqueError::Empty => write!(f, "the critic returned nothing"),
        }
    }
}

impl std::error::Error for CritiqueError {}

/// Splits a reply into bullet points: `- ` / `* ` lines, trimmed. A reply with no bullet
/// markers falls back to its whole trimmed text as one point, so a critique is never lost.
fn parse_bullets(text: &str) -> Vec<String> {
    let bullets: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            t.strip_prefix("- ")
                .or_else(|| t.strip_prefix("* "))
                .map(|b| b.trim().to_owned())
        })
        .filter(|b| !b.is_empty())
        .collect();
    if bullets.is_empty() {
        let whole = text.trim();
        if whole.is_empty() {
            Vec::new()
        } else {
            vec![whole.to_owned()]
        }
    } else {
        bullets
    }
}

/// Reviews `doc` (optionally against a stated `goal`) and returns the critique as bullet
/// points.
///
/// # Errors
/// [`CritiqueError::Provider`] on a provider failure, or [`CritiqueError::Empty`] if the
/// reply is blank.
pub fn critique_patch(
    provider: &dyn AiProvider,
    registry: &Registry,
    config: &ProviderConfig,
    doc: &ProjectDoc,
    goal: Option<&str>,
) -> Result<Vec<String>, CritiqueError> {
    let system = format!(
        "{CRITIC_PREAMBLE}\n\n# Available nodes\n\n{}",
        node_reference(registry)
    );
    let goal_line = match goal {
        Some(g) => format!("Goal: {g}\n\n"),
        None => String::new(),
    };
    let user = format!("{goal_line}Review this patch:\n\n{}", doc.to_prompt());

    let request = ChatRequest::new(config, system, vec![Message::user(user)]);
    let reply = provider
        .complete(&request)
        .map_err(CritiqueError::Provider)?;

    let bullets = parse_bullets(&reply.content);
    if bullets.is_empty() {
        return Err(CritiqueError::Empty);
    }
    Ok(bullets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ScriptedProvider;
    use seclab_core::{Graph, GraphNode, Port, PortId, SignalType};

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
    fn parse_bullets_handles_dash_star_and_prose() {
        assert_eq!(
            parse_bullets("- one\n* two\n  - three"),
            vec!["one", "two", "three"]
        );
        // No markers → the whole thing is one point.
        assert_eq!(parse_bullets("just prose"), vec!["just prose"]);
        assert!(parse_bullets("   ").is_empty());
    }

    #[test]
    fn critique_grounds_the_prompt_and_returns_bullets() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&[
            "- The gain is redundant if you only need level; the oscillator already outputs full scale.\n- Otherwise this is minimal.",
        ]);

        let bullets = critique_patch(
            &provider,
            &registry,
            &config(),
            &osc_gain(),
            Some("a test tone"),
        )
        .unwrap();
        assert_eq!(bullets.len(), 2);
        assert!(bullets[0].contains("redundant"));

        let request = &provider.requests()[0];
        assert!(request.system.contains("Occam"), "critic framing");
        assert!(
            request.system.contains("oscillator [generator]"),
            "grounded"
        );
        assert!(request.messages[0].content.contains("Goal: a test tone"));
        assert!(request.messages[0].content.contains("node 0 oscillator"));
    }

    #[test]
    fn a_prose_reply_becomes_a_single_bullet() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&["This patch is already minimal."]);
        let bullets = critique_patch(&provider, &registry, &config(), &osc_gain(), None).unwrap();
        assert_eq!(bullets, vec!["This patch is already minimal."]);
        // No goal line when none is given.
        assert!(!provider.requests()[0].messages[0].content.contains("Goal:"));
    }

    #[test]
    fn an_empty_reply_is_an_error() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&["   "]);
        assert_eq!(
            critique_patch(&provider, &registry, &config(), &osc_gain(), None),
            Err(CritiqueError::Empty)
        );
    }

    #[test]
    fn a_provider_failure_propagates() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::new(vec![Err(AiError::MissingKey)]);
        assert_eq!(
            critique_patch(&provider, &registry, &config(), &osc_gain(), None),
            Err(CritiqueError::Provider(AiError::MissingKey))
        );
    }
}
