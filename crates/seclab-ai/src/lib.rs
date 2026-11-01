//! `seclab-ai` — the AI assistant's grounding and output contract.
//!
//! Epic 12. This crate assembles the system prompt the assistant runs under: a
//! versioned, in-repo pre-prompt (role, grounding rules, the RON `ProjectDoc` output
//! contract, refusal rules) stitched together with the live, auto-generated node
//! reference ([`seclab_nodes::node_reference`]) and — when editing — the current patch in
//! the compact prompt form ([`seclab_project::ProjectDoc::to_prompt`]).
//!
//! The prompt assets are the single source of the assistant's behaviour; keeping them
//! here, versioned and tested, means the contract shown to the model never drifts from
//! what the app can actually parse and validate.

pub mod anthropic;
pub mod budget;
pub mod compress;
pub mod converge;
pub mod critic;
pub mod design;
pub mod diff;
pub mod edit_session;
pub mod explain;
pub mod generate;
pub mod github;
pub mod http;
pub mod inspect;
pub mod keystore;
pub mod local;
pub mod openai;
pub mod persist;
pub mod privacy;
pub mod prompt;
pub mod provider;
pub mod repair;
pub mod report;
pub mod settings;
pub mod suggest;

pub use anthropic::{build_request_body, parse_response, AnthropicProvider};
pub use budget::{
    estimate_cost_usd, estimate_tokens, estimate_usage, BudgetCheck, BudgetGuard, Usage,
};
pub use compress::{distill_history, graph_summary};
pub use converge::{converge, ConvergeError, Converged, Verdict};
pub use critic::{critique_patch, CritiqueError};
pub use design::{DesignError, DesignSession, DesignStep};
pub use diff::{PatchDiff, Retype};
pub use edit_session::{EditSession, EditState, Turn, TurnStatus};
pub use explain::{explain_node, explain_patch, ExplainError};
pub use generate::{edit_patch, generate_patch, EditOutcome, GenerateError, GenerateOutcome};
#[cfg(feature = "keychain")]
pub use github::KeychainTokenStore;
pub use github::{
    create_issue, issues_url, AccessToken, AuthError, CreatedIssue, DeviceCode, DeviceFlow,
    MemoryTokenStore, PollOutcome, TokenStore, ACCESS_TOKEN_URL, DEVICE_CODE_URL,
    DEVICE_GRANT_TYPE, REPORT_CLIENT_ID, REPORT_REPO_NAME, REPORT_REPO_OWNER, REPORT_SCOPE,
};
#[cfg(feature = "net")]
pub use http::ReqwestTransport;
pub use http::{Header, HttpReply, HttpTransport};
pub use inspect::{inspect_request, RequestInspection, Segment, SessionInspector};
pub use keystore::{KeyStore, KeyStoreError, MemoryKeyStore};
pub use local::{LocalBackend, LocalProvider};
// The OpenAI-compatible codec (`build_request_body` / `parse_response`) is reached as
// `openai::…` to avoid colliding with the flat `anthropic::…` re-exports above; the
// provider type is the primary entry point, so it is re-exported here.
pub use openai::OpenAiProvider;
pub use persist::{load_settings, save_settings, settings_path, SettingsIoError, SETTINGS_FILE};
pub use privacy::{preview_request, GuardedProvider, PrivacySettings};
pub use prompt::{build_system_prompt, EXAMPLE_PATCH_RON, PROMPT_VERSION, SYSTEM_PREAMBLE};
pub use provider::{
    AiError, AiProvider, ChatRequest, ChatResponse, Message, ProviderConfig, ProviderKind, Role,
    ScriptedProvider, DEFAULT_MAX_TOKENS, DEFAULT_MODEL,
};
pub use repair::{repair_patch, RepairError, RepairOutcome};
pub use report::{build_issue, scrub, IssuePayload, Metadata, ReportDraft, ReportKind, REDACTION};
pub use settings::{AssistantSettings, SendBlocker};
pub use suggest::{
    autofix, compatible_consumers, free_outputs, suggest_next, EdgeDefect, Fix, FreeOutput,
    NextNode,
};
