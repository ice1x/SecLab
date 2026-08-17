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

pub mod budget;
pub mod compress;
pub mod converge;
pub mod critic;
pub mod design;
pub mod diff;
pub mod explain;
pub mod generate;
pub mod keystore;
pub mod privacy;
pub mod prompt;
pub mod provider;
pub mod repair;
pub mod suggest;

pub use budget::{
    estimate_cost_usd, estimate_tokens, estimate_usage, BudgetCheck, BudgetGuard, Usage,
};
pub use compress::{distill_history, graph_summary};
pub use converge::{converge, ConvergeError, Converged, Verdict};
pub use critic::{critique_patch, CritiqueError};
pub use design::{DesignError, DesignSession, DesignStep};
pub use diff::{PatchDiff, Retype};
pub use explain::{explain_node, explain_patch, ExplainError};
pub use generate::{edit_patch, generate_patch, EditOutcome, GenerateError, GenerateOutcome};
pub use keystore::{KeyStore, KeyStoreError, MemoryKeyStore};
pub use privacy::{preview_request, GuardedProvider, PrivacySettings};
pub use prompt::{build_system_prompt, EXAMPLE_PATCH_RON, PROMPT_VERSION, SYSTEM_PREAMBLE};
pub use provider::{
    AiError, AiProvider, ChatRequest, ChatResponse, Message, ProviderConfig, ProviderKind, Role,
    ScriptedProvider, DEFAULT_MAX_TOKENS, DEFAULT_MODEL,
};
pub use repair::{repair_patch, RepairError, RepairOutcome};
pub use suggest::{
    autofix, compatible_consumers, free_outputs, suggest_next, EdgeDefect, Fix, FreeOutput,
    NextNode,
};
