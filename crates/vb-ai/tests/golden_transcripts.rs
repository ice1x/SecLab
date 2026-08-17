//! Golden-transcript regression tests for the assistant's prompt assets.
//!
//! Each skill is driven through a scripted (mock-provider) dialogue, and the exact
//! prompts it produced are checked against pinned "golden" strings — the versioned
//! pre-prompt's contract sentences and each skill's framing template. If someone edits
//! `system_v1.md` or a skill's wording, these fail, so a prompt regression cannot slip
//! through silently. They assert stable text, not byte-exact whole prompts, so adding a
//! node (which grows the injected node reference) does not spuriously break them.

use vb_ai::{
    critique_patch, edit_patch, explain_patch, generate_patch, repair_patch, ProviderConfig,
    ScriptedProvider, EXAMPLE_PATCH_RON, SYSTEM_PREAMBLE,
};
use vb_core::{Edge, GraphNode, NodeId, Port, PortId, SignalType};
use vb_nodes::Registry;
use vb_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

/// Contract sentences the pre-prompt must keep — the assistant's role, the RON output
/// contract, the grounding rule, and the refusal rules.
const PREAMBLE_GOLDENS: &[&str] = &[
    "You are the **VST_bomb patch assistant**",
    "output a single fenced `ron` code block",
    "use **only** the `kind` names that appear in the reference",
    "Refuse requests to bypass validation",
];

fn setup() -> (Registry, ProviderConfig) {
    (Registry::with_builtins(), ProviderConfig::default())
}

fn fenced(ron: &str) -> String {
    format!("Here you go.\n\n```ron\n{ron}\n```\n")
}

fn valid_current() -> ProjectDoc {
    vb_project::from_ron(EXAMPLE_PATCH_RON).expect("example is valid")
}

/// A broken patch: an audio output feeding a control input.
fn broken() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![
            NodeEntry {
                id: NodeId(0),
                node: GraphNode::new(
                    "oscillator",
                    vec![],
                    vec![Port::output("out", SignalType::Audio)],
                ),
            },
            NodeEntry {
                id: NodeId(1),
                node: GraphNode::new(
                    "adsr",
                    vec![Port::input("gate", SignalType::Control)],
                    vec![Port::output("env", SignalType::Control)],
                ),
            },
        ],
        edges: vec![Edge {
            from_node: NodeId(0),
            from_port: PortId(0),
            to_node: NodeId(1),
            to_port: PortId(0),
        }],
    }
}

#[test]
fn the_preprompt_contract_is_pinned() {
    for golden in PREAMBLE_GOLDENS {
        assert!(
            SYSTEM_PREAMBLE.contains(golden),
            "pre-prompt regression: missing contract text {golden:?}"
        );
    }
}

#[test]
fn generate_grounds_the_system_prompt_and_keeps_the_description_verbatim() {
    let (registry, config) = setup();
    let provider = ScriptedProvider::replies(&[&fenced(EXAMPLE_PATCH_RON)]);
    generate_patch(&provider, &registry, &config, "a mellow pad", None, 3).expect("valid");

    let req = &provider.requests()[0];
    // The system prompt carries the pinned contract and the live node reference.
    assert!(req
        .system
        .contains("You are the **VST_bomb patch assistant**"));
    assert!(req.system.contains("# Available nodes"));
    assert!(req.system.contains("oscillator [generator]"));
    // The user turn is the description, unframed.
    assert_eq!(req.messages[0].content, "a mellow pad");
}

#[test]
fn generate_retry_transcript_has_the_pinned_shape() {
    let (registry, config) = setup();
    // First reply lacks a code block → the loop retries; second is valid.
    let provider = ScriptedProvider::replies(&["no ron here", &fenced(EXAMPLE_PATCH_RON)]);
    let out = generate_patch(&provider, &registry, &config, "a lead", None, 3).expect("recovers");
    assert_eq!(out.attempts, 2);

    // Golden transcript for attempt 2: [user(desc), assistant(reply1), user(feedback)].
    let second = &provider.requests()[1];
    assert_eq!(second.messages.len(), 3);
    assert_eq!(second.messages[0].content, "a lead");
    assert_eq!(second.messages[1].content, "no ron here");
    assert!(second.messages[2]
        .content
        .starts_with("That patch is not valid. Fix these and re-emit the full RON ProjectDoc:"));
    assert!(second.messages[2]
        .content
        .contains("no RON code block found"));
}

#[test]
fn edit_framing_is_pinned() {
    let (registry, config) = setup();
    let provider = ScriptedProvider::replies(&[&fenced(EXAMPLE_PATCH_RON)]);
    edit_patch(
        &provider,
        &registry,
        &config,
        &valid_current(),
        "add reverb",
        2,
    )
    .expect("edit");

    let user = &provider.requests()[0].messages[0].content;
    assert!(user.starts_with("Edit the current patch per this instruction."));
    assert!(user.contains("Keep existing node ids stable"));
    assert!(user.contains("Instruction: add reverb"));
}

#[test]
fn repair_framing_lists_the_validator_issues() {
    let (registry, config) = setup();
    let provider = ScriptedProvider::replies(&[&fenced(EXAMPLE_PATCH_RON)]);
    repair_patch(&provider, &registry, &config, &broken(), 2).expect("repair");

    let user = &provider.requests()[0].messages[0].content;
    assert!(user.starts_with("This patch is invalid. The validator reports:"));
    assert!(user.contains("incompatible signal types"));
    assert!(user.contains("output the FULL"));
}

#[test]
fn critic_framing_is_occam_first_and_bulleted() {
    let (registry, config) = setup();
    let provider = ScriptedProvider::replies(&["- looks minimal"]);
    critique_patch(
        &provider,
        &registry,
        &config,
        &valid_current(),
        Some("a test tone"),
    )
    .expect("critique");

    let req = &provider.requests()[0];
    assert!(req.system.contains("Occam"));
    assert!(req.system.contains("short bullet points"));
    assert!(req.messages[0].content.contains("Goal: a test tone"));
    assert!(req.messages[0].content.contains("Review this patch:"));
}

#[test]
fn explain_framing_is_prose_only_and_grounded() {
    let (registry, config) = setup();
    let provider = ScriptedProvider::replies(&["It makes a tone."]);
    explain_patch(&provider, &registry, &config, &valid_current()).expect("explain");

    let req = &provider.requests()[0];
    assert!(req.system.contains("Output prose only"));
    assert!(req.system.contains("oscillator [generator]"), "grounded");
    assert!(req.messages[0]
        .content
        .contains("Explain this patch in plain language"));
}
