//! egui layout for the AI assistant chat panel (backlog 000108).
//!
//! A thin rendering shell over the tested [`vb_ai::EditSession`] view-model: a transcript of
//! past requests, the pending proposal's diff preview with apply/reject buttons, and an input
//! box to submit a new request. All the request → proposal → apply/reject logic lives (and is
//! unit-tested) in `EditSession`; this file only turns that state into widgets and reports the
//! user's intent back to the caller.
//!
//! The panel is deliberately **provider-free**: it never calls the model itself. It returns an
//! [`AssistantAction`] describing what the user asked for, and the caller — which owns the
//! [`vb_ai::AiProvider`], [`vb_nodes::Registry`] and config — drives the actual
//! [`vb_ai::EditSession::submit`] / [`apply`](vb_ai::EditSession::apply) /
//! [`reject`](vb_ai::EditSession::reject) call. That keeps the HTTP/provider stack out of the
//! render layer and lets the app wrap `apply` as one undoable command.
//!
//! This whole module (and the `vb-ai` dependency it needs) sits behind the off-by-default,
//! native-only `assistant` feature, so the wasm and plugin builds never pull in the provider
//! stack — see the crate's `Cargo.toml`.

use egui::Ui;
use vb_ai::{EditSession, EditState, TurnStatus};

/// What the user asked the panel to do this frame. The caller performs the matching
/// [`EditSession`] call; the panel itself touches no provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantAction {
    /// Submit a new natural-language request (the text the user typed).
    Submit(String),
    /// Apply the pending proposal — it becomes the live patch.
    Apply,
    /// Reject the pending proposal — the live patch is left unchanged.
    Reject,
}

/// Renders the assistant panel into `ui` from the read-only `session` state, using `draft` as
/// the (app-owned) text-input buffer. Returns `Some(action)` when the user clicks send / apply
/// / reject this frame, otherwise `None`. On [`AssistantAction::Submit`] the `draft` buffer is
/// cleared (its contents move into the action).
pub fn assistant_panel_ui(
    ui: &mut Ui,
    session: &EditSession,
    draft: &mut String,
) -> Option<AssistantAction> {
    ui.label("assistant");
    ui.separator();

    let mut action = None;

    // Transcript: each past request with how it resolved and the assistant's explanation.
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if session.transcript().is_empty() {
                ui.weak("ask the assistant to edit the patch…");
            }
            for turn in session.transcript() {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("you:");
                    ui.label(&turn.request);
                });
                match &turn.status {
                    TurnStatus::Pending => {
                        ui.weak("proposed — awaiting your decision");
                    }
                    TurnStatus::Applied => {
                        ui.weak("applied");
                    }
                    TurnStatus::Rejected => {
                        ui.weak("rejected");
                    }
                    TurnStatus::Superseded => {
                        ui.weak("superseded by a later request");
                    }
                    TurnStatus::Failed(reason) => {
                        ui.colored_label(egui::Color32::from_rgb(0xd0, 0x50, 0x50), reason);
                    }
                }
                if !turn.explanation.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong("assistant:");
                        ui.label(&turn.explanation);
                    });
                }
                ui.separator();
            }
        });

    // Pending proposal: preview its diff and offer apply / reject.
    if session.state() == EditState::Proposed {
        if let Some(outcome) = session.pending() {
            ui.label("proposed change:");
            ui.code(outcome.diff.summary());
            ui.horizontal(|ui| {
                if ui.button("apply").clicked() {
                    action = Some(AssistantAction::Apply);
                }
                if ui.button("reject").clicked() {
                    action = Some(AssistantAction::Reject);
                }
            });
        }
    }

    // Input row: type a request and send it. Sending supersedes any pending proposal (that is
    // the view-model's contract), so the button stays enabled whenever there is text.
    ui.separator();
    ui.horizontal(|ui| {
        let text_edit = ui.add(
            egui::TextEdit::singleline(draft)
                .hint_text("describe an edit…")
                .desired_width(f32::INFINITY),
        );
        let submit_via_enter =
            text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let send = ui.add_enabled(!draft.trim().is_empty(), egui::Button::new("send"));
        if (send.clicked() || submit_via_enter) && !draft.trim().is_empty() {
            action = Some(AssistantAction::Submit(std::mem::take(draft)));
        }
    });

    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_ai::provider::{ProviderConfig, ScriptedProvider};
    use vb_ai::EXAMPLE_PATCH_RON;
    use vb_nodes::Registry;
    use vb_project::ProjectDoc;

    fn empty_doc() -> ProjectDoc {
        vb_project::from_ron("(version: 1, nodes: [], edges: [])").expect("empty doc")
    }

    fn reply_with(ron: &str) -> String {
        format!("Here is the edited patch.\n\n```ron\n{ron}\n```\n")
    }

    /// Runs `assistant_panel_ui` once inside a headless egui context — no window, no display.
    /// A smoke test: it proves the layout code builds a frame without panicking (visual
    /// correctness is verified by a human). Returns whatever the panel returned.
    fn run_once(session: &EditSession, draft: &mut String) -> Option<AssistantAction> {
        let ctx = egui::Context::default();
        let mut out = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = assistant_panel_ui(ui, session, draft);
            });
        });
        out
    }

    #[test]
    fn an_idle_empty_session_renders_without_panicking() {
        let session = EditSession::new(empty_doc());
        let mut draft = String::new();
        // No interaction this frame → nothing chosen, but the frame must build cleanly.
        assert!(run_once(&session, &mut draft).is_none());
    }

    #[test]
    fn a_proposed_session_renders_its_diff_and_buttons() {
        let registry = Registry::with_builtins();
        let config = ProviderConfig::default();
        let provider = ScriptedProvider::replies(&[&reply_with(EXAMPLE_PATCH_RON)]);
        let mut session = EditSession::new(empty_doc());
        session
            .submit(&provider, &registry, &config, "add a filter", 3)
            .expect("proposes an edit");
        assert_eq!(session.state(), EditState::Proposed);

        let mut draft = String::new();
        // The diff-preview + apply/reject branch must render cleanly.
        assert!(run_once(&session, &mut draft).is_none());
    }

    #[test]
    fn a_failed_session_renders_its_error() {
        let registry = Registry::with_builtins();
        let config = ProviderConfig::default();
        // A reply with no RON block never converges → the turn fails.
        let provider = ScriptedProvider::replies(&["only prose", "still prose"]);
        let mut session = EditSession::new(empty_doc());
        let _ = session.submit(&provider, &registry, &config, "do something", 2);
        assert_eq!(session.state(), EditState::Failed);

        let mut draft = String::new();
        // The colored-error branch must render cleanly.
        assert!(run_once(&session, &mut draft).is_none());
    }
}
