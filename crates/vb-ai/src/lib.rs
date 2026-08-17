//! `vb-ai` — the AI assistant's grounding and output contract.
//!
//! Epic 12. This crate assembles the system prompt the assistant runs under: a
//! versioned, in-repo pre-prompt (role, grounding rules, the RON `ProjectDoc` output
//! contract, refusal rules) stitched together with the live, auto-generated node
//! reference ([`vb_nodes::node_reference`]) and — when editing — the current patch in
//! the compact prompt form ([`vb_project::ProjectDoc::to_prompt`]).
//!
//! The prompt assets are the single source of the assistant's behaviour; keeping them
//! here, versioned and tested, means the contract shown to the model never drifts from
//! what the app can actually parse and validate.

pub mod prompt;

pub use prompt::{build_system_prompt, EXAMPLE_PATCH_RON, PROMPT_VERSION, SYSTEM_PREAMBLE};
