//! The "Report a problem / feature request" button and dialog (000255).
//!
//! The thin egui view over [`seclab_ai::ReportSession`]: a toolbar button opens a dialog
//! where the user picks bug/feature, types a title + description, signs in to GitHub (the
//! device flow, run on a worker thread so the minutes-long authorization never freezes the
//! UI), reviews exactly what will be sent, and submits — the report lands as a GitHub issue
//! and the dialog links straight to it.
//!
//! Reporting is **auth-gated**: the Send button stays disabled until the user is signed in
//! (no account → no report). The network work (device-flow sign-in, issue creation) needs a
//! real HTTP stack + the OS keychain, so it is compiled only under the `assistant-net`
//! feature — exactly like the assistant. In the base build the dialog still opens and is
//! unit-tested, but the sign-in/submit actions report that the network build is required.
//!
//! The session's own safety rules live in [`seclab_ai::ReportSession`]: the signed-in token
//! is always scrubbed out of the payload, and a 401 clears it so the UI re-prompts a
//! sign-in. This module only drives that state machine and renders it.

use std::sync::mpsc;
use std::thread::JoinHandle;

use egui::{self, Ui};

use seclab_ai::{
    build_issue, CreatedIssue, DeviceCode, Metadata, ReportDraft, ReportKind, TokenStore,
};

use crate::log_buffer::LogBuffer;

/// How many recent log lines a report carries.
const LOG_TAIL_LINES: usize = 200;

#[cfg(feature = "assistant-net")]
use seclab_ai::{AuthError, PollOutcome, ReportSession, ReqwestTransport};

/// Which stage of the report flow the dialog is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Editing the report (and signing in, if not already).
    Form,
    /// A device code is shown; a worker polls GitHub for authorization.
    SigningIn,
    /// A worker is creating the issue. Reached only on the network path.
    #[cfg_attr(not(feature = "assistant-net"), allow(dead_code))]
    Submitting,
    /// The issue was created; the dialog links to it.
    Done,
}

/// A message from a worker thread back to the UI.
///
/// The variants are produced by the worker threads, which exist only under `assistant-net`;
/// the base `assistant` build still compiles the view-model (and its tests) but never
/// constructs them outside tests, hence the conditional `allow`.
#[cfg_attr(not(feature = "assistant-net"), allow(dead_code))]
enum ReportMsg {
    /// The device/user code to show while authorization is pending (non-terminal: the same
    /// worker keeps polling and will send a terminal message next).
    Code(DeviceCode),
    /// Sign-in completed and the token was stored.
    SignedIn,
    /// Sign-in failed or was declined/expired.
    SignInFailed(String),
    /// The issue was created.
    Submitted(CreatedIssue),
    /// Issue creation failed; `reauth` is true when the token was rejected (401).
    SubmitFailed { message: String, reauth: bool },
}

/// The report dialog's state and its worker plumbing.
///
/// Self-contained: owns the reporter's [`TokenStore`] (the OS keychain under
/// `assistant-net`, else an in-session store) and the in-flight worker, so [`EditorApp`]
/// only has to call [`ReportHost::poll`] each frame and [`ReportHost::ui`].
///
/// [`EditorApp`]: crate::app::EditorApp
pub struct ReportHost {
    /// Whether the dialog is open.
    open: bool,
    phase: Phase,
    kind: ReportKind,
    title: String,
    description: String,
    /// The device code shown while [`Phase::SigningIn`].
    device: Option<DeviceCode>,
    /// The created issue shown while [`Phase::Done`].
    issue: Option<CreatedIssue>,
    /// Last status/error line.
    status: Option<String>,
    /// Token storage (keychain under `assistant-net`), read for the signed-in state and
    /// cleared on sign-out.
    token: Box<dyn TokenStore>,
    /// The shared log sink whose tail is attached to a report (000256). Empty by default
    /// until the host shares its buffer via [`ReportHost::set_log`].
    log: LogBuffer,
    rx: Option<mpsc::Receiver<ReportMsg>>,
    worker: Option<JoinHandle<()>>,
}

impl Default for ReportHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportHost {
    /// A closed dialog with the keychain-backed token store under `assistant-net`, else an
    /// in-session store (enough to drive the headless tests and the base build).
    #[must_use]
    pub fn new() -> Self {
        #[cfg(feature = "assistant-net")]
        let token: Box<dyn TokenStore> = Box::new(seclab_ai::KeychainTokenStore::new("SecLab"));
        #[cfg(not(feature = "assistant-net"))]
        let token: Box<dyn TokenStore> = Box::new(seclab_ai::MemoryTokenStore::new());
        Self::with_token_store(token)
    }

    /// A closed dialog over an explicit token store. Lets the tests inject a
    /// [`seclab_ai::MemoryTokenStore`] so they never touch the real OS keychain, whatever
    /// feature set the build unified to.
    #[must_use]
    fn with_token_store(token: Box<dyn TokenStore>) -> Self {
        Self {
            open: false,
            phase: Phase::Form,
            kind: ReportKind::Bug,
            title: String::new(),
            description: String::new(),
            device: None,
            issue: None,
            status: None,
            token,
            log: LogBuffer::new(1),
            rx: None,
            worker: None,
        }
    }

    /// Shares the app's log buffer so reports carry a tail of recent log lines.
    pub fn set_log(&mut self, log: LogBuffer) {
        self.log = log;
    }

    /// Opens the dialog. Reopening after a finished report starts a fresh form.
    pub fn open(&mut self) {
        if self.phase == Phase::Done {
            self.reset_form();
        }
        self.open = true;
    }

    /// Whether a worker request is in flight.
    fn in_flight(&self) -> bool {
        self.rx.is_some()
    }

    /// Whether a token is stored (the user is signed in). A store read failure counts as
    /// signed out — the UI then simply offers sign-in.
    fn signed_in(&self) -> bool {
        self.token.is_signed_in().unwrap_or(false)
    }

    /// Clears the draft back to an empty bug report.
    fn reset_form(&mut self) {
        self.phase = Phase::Form;
        self.kind = ReportKind::Bug;
        self.title.clear();
        self.description.clear();
        self.device = None;
        self.issue = None;
        self.status = None;
    }

    /// Signs out: clears the stored token.
    pub fn sign_out(&mut self) {
        let _ = self.token.clear();
        self.status = Some("signed out".to_owned());
    }

    /// Consumes a worker message if one has arrived, advancing the dialog. Returns whether
    /// state changed. Call once per frame.
    pub fn poll(&mut self) -> bool {
        let msg = match self.rx.as_ref() {
            Some(rx) => rx.try_recv(),
            None => return false,
        };
        match msg {
            Ok(message) => {
                // `Code` is non-terminal: the same worker keeps polling, so keep the channel
                // open for its follow-up. Every other message ends the worker.
                let terminal = !matches!(message, ReportMsg::Code(_));
                if terminal {
                    self.rx = None;
                    if let Some(worker) = self.worker.take() {
                        let _ = worker.join();
                    }
                }
                self.apply_msg(message);
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                if matches!(self.phase, Phase::SigningIn | Phase::Submitting) {
                    self.phase = Phase::Form;
                }
                self.status = Some("the background task stopped unexpectedly".to_owned());
                true
            }
        }
    }

    /// Applies one worker message to the dialog state (the pure half of [`poll`], unit-tested
    /// directly).
    fn apply_msg(&mut self, msg: ReportMsg) {
        match msg {
            ReportMsg::Code(device) => {
                self.device = Some(device);
                self.phase = Phase::SigningIn;
            }
            ReportMsg::SignedIn => {
                self.phase = Phase::Form;
                self.device = None;
                self.status = Some("signed in to GitHub".to_owned());
            }
            ReportMsg::SignInFailed(e) => {
                self.phase = Phase::Form;
                self.device = None;
                self.status = Some(format!("sign-in failed: {e}"));
            }
            ReportMsg::Submitted(issue) => {
                self.issue = Some(issue);
                self.status = None;
                self.phase = Phase::Done;
            }
            ReportMsg::SubmitFailed { message, reauth } => {
                self.phase = Phase::Form;
                // On a 401 the session already dropped the stale token, so the form now shows
                // the signed-out state and prompts a fresh sign-in.
                self.status = Some(if reauth {
                    format!("{message} — please sign in again")
                } else {
                    message
                });
            }
        }
    }

    /// Starts the GitHub device-flow sign-in on a worker thread (or reports that the network
    /// build is required).
    fn begin_sign_in(&mut self) {
        if self.in_flight() {
            return;
        }
        #[cfg(feature = "assistant-net")]
        {
            let (tx, rx) = mpsc::channel();
            let worker = std::thread::spawn(move || sign_in_worker(&tx));
            self.rx = Some(rx);
            self.worker = Some(worker);
            self.phase = Phase::SigningIn;
            self.status = None;
        }
        #[cfg(not(feature = "assistant-net"))]
        {
            self.status = Some("sign-in needs the network build (assistant-net)".to_owned());
        }
    }

    /// Validates the draft and submits it on a worker thread.
    fn begin_submit(&mut self, git_sha: &str) {
        if self.in_flight() {
            return;
        }
        if self.title.trim().is_empty() {
            self.status = Some("please enter a short title".to_owned());
            return;
        }
        if !self.signed_in() {
            self.status = Some("sign in to GitHub first".to_owned());
            return;
        }
        let draft = ReportDraft::new(self.kind, self.title.trim(), self.description.trim());
        let meta = Metadata::current(Some(git_sha.to_owned()));
        let logs = self.log.tail(LOG_TAIL_LINES);
        #[cfg(feature = "assistant-net")]
        {
            let (tx, rx) = mpsc::channel();
            let worker = std::thread::spawn(move || submit_worker(&tx, draft, meta, logs));
            self.rx = Some(rx);
            self.worker = Some(worker);
            self.phase = Phase::Submitting;
            self.status = None;
        }
        #[cfg(not(feature = "assistant-net"))]
        {
            let _ = (draft, meta, logs);
            self.status = Some("sending needs the network build (assistant-net)".to_owned());
        }
    }

    /// The Markdown preview of exactly what would be sent (title + scrubbed body).
    fn preview(&self, git_sha: &str) -> String {
        let draft = ReportDraft::new(self.kind, self.title.trim(), self.description.trim());
        let meta = Metadata::current(Some(git_sha.to_owned()));
        build_issue(&draft, &meta, &self.log.tail(LOG_TAIL_LINES), &[]).preview()
    }

    /// Draws the toolbar button that opens the dialog.
    pub fn toolbar_button(&mut self, ui: &mut Ui) {
        if ui
            .button("🐞 Report")
            .on_hover_text("Report a problem or request a feature")
            .clicked()
        {
            self.open();
        }
    }

    /// Draws the dialog window. `git_sha` stamps the environment block.
    pub fn ui(&mut self, ui: &mut Ui, git_sha: &str) {
        if !self.open {
            return;
        }
        // Keep the frame ticking while a worker runs, so `poll` actually fires.
        if self.in_flight() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }

        let mut open = self.open;
        egui::Window::new("Report a problem / feature request")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .show(ui.ctx(), |ui| match self.phase {
                Phase::Form => self.form_ui(ui, git_sha),
                Phase::SigningIn => self.signing_in_ui(ui),
                Phase::Submitting => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("creating the issue…");
                    });
                }
                Phase::Done => self.done_ui(ui),
            });
        self.open = open;
    }

    /// The main form: kind, title, description, sign-in state, preview, send.
    fn form_ui(&mut self, ui: &mut Ui, git_sha: &str) {
        ui.horizontal(|ui| {
            ui.label("Type:");
            ui.selectable_value(&mut self.kind, ReportKind::Bug, "🐞 Bug");
            ui.selectable_value(&mut self.kind, ReportKind::Feature, "✨ Feature");
        });
        ui.add_space(4.0);
        ui.label("Title");
        ui.add(egui::TextEdit::singleline(&mut self.title).hint_text("short summary"));
        ui.add_space(4.0);
        ui.label("Description");
        ui.add(
            egui::TextEdit::multiline(&mut self.description)
                .hint_text("what happened, steps, what you expected")
                .desired_rows(4),
        );

        ui.separator();
        let signed_in = self.signed_in();
        ui.horizontal(|ui| {
            if signed_in {
                ui.label("✓ signed in to GitHub");
                if ui.button("Sign out").clicked() {
                    self.sign_out();
                }
            } else {
                ui.label("Sign in with GitHub to send a report.");
                if ui.button("Sign in").clicked() {
                    self.begin_sign_in();
                }
            }
        });

        let mut preview = self.preview(git_sha);
        ui.collapsing("What will be sent?", |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut preview)
                    .desired_rows(8)
                    .interactive(false),
            );
            ui.weak("Logs and secrets (API keys, tokens) are scrubbed before sending.");
        });

        ui.separator();
        ui.horizontal(|ui| {
            let can_send = signed_in && !self.title.trim().is_empty();
            if ui
                .add_enabled(can_send, egui::Button::new("Send report"))
                .on_disabled_hover_text(if signed_in {
                    "enter a title first"
                } else {
                    "sign in first"
                })
                .clicked()
            {
                self.begin_submit(git_sha);
            }
            if ui.button("Cancel").clicked() {
                self.open = false;
            }
        });
        if let Some(status) = &self.status {
            ui.separator();
            ui.weak(status);
        }
    }

    /// The sign-in-pending view: the user code + a link to github.com/login/device.
    fn signing_in_ui(&mut self, ui: &mut Ui) {
        ui.label("To sign in, open the link and enter this code:");
        ui.add_space(6.0);
        if let Some(device) = &self.device {
            ui.horizontal(|ui| {
                ui.label("Code:");
                ui.monospace(&device.user_code);
            });
            ui.hyperlink_to("Open github.com/login/device", &device.verification_uri);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak("waiting for you to authorize…");
        });
        if ui.button("Cancel").clicked() {
            // Drop the worker handle; its send will just be ignored.
            self.rx = None;
            self.worker = None;
            self.device = None;
            self.phase = Phase::Form;
        }
    }

    /// The success view: a link to the created issue.
    fn done_ui(&mut self, ui: &mut Ui) {
        ui.label("✓ Thanks — your report was filed:");
        if let Some(issue) = &self.issue {
            ui.hyperlink_to(format!("View issue #{}", issue.number), &issue.html_url);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("New report").clicked() {
                self.reset_form();
            }
            if ui.button("Close").clicked() {
                self.open = false;
            }
        });
    }
}

/// The sign-in worker body: request a device code, report it, then poll until the user
/// authorizes (or the code is declined/expires). Runs its own transport + keychain store on
/// a background thread so the UI never blocks.
#[cfg(feature = "assistant-net")]
fn sign_in_worker(tx: &mpsc::Sender<ReportMsg>) {
    let transport = match ReqwestTransport::new(std::time::Duration::from_secs(60)) {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(ReportMsg::SignInFailed(e.to_string()));
            return;
        }
    };
    let mut store = seclab_ai::KeychainTokenStore::new("SecLab");
    let mut session = ReportSession::new(&mut store);
    let device = match session.begin_sign_in(&transport) {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.send(ReportMsg::SignInFailed(e.to_string()));
            return;
        }
    };
    let device_code = device.device_code.clone();
    let mut interval = device.interval.max(1);
    let _ = tx.send(ReportMsg::Code(device));

    loop {
        std::thread::sleep(std::time::Duration::from_secs(interval));
        match session.poll_sign_in(&transport, &device_code) {
            Ok(PollOutcome::Authorized(_)) => {
                let _ = tx.send(ReportMsg::SignedIn);
                return;
            }
            Ok(PollOutcome::Pending) => {}
            Ok(PollOutcome::SlowDown { interval: next }) => interval = next.max(interval + 5),
            Ok(PollOutcome::Denied) => {
                let _ = tx.send(ReportMsg::SignInFailed(
                    "authorization was declined".to_owned(),
                ));
                return;
            }
            Ok(PollOutcome::Expired) => {
                let _ = tx.send(ReportMsg::SignInFailed(
                    "the code expired — try again".to_owned(),
                ));
                return;
            }
            Err(e) => {
                let _ = tx.send(ReportMsg::SignInFailed(e.to_string()));
                return;
            }
        }
    }
}

/// The submit worker body: create the issue as the signed-in user. Runs its own transport +
/// keychain store on a background thread.
#[cfg(feature = "assistant-net")]
fn submit_worker(tx: &mpsc::Sender<ReportMsg>, draft: ReportDraft, meta: Metadata, logs: String) {
    let transport = match ReqwestTransport::new(std::time::Duration::from_secs(60)) {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(ReportMsg::SubmitFailed {
                message: e.to_string(),
                reauth: false,
            });
            return;
        }
    };
    let mut store = seclab_ai::KeychainTokenStore::new("SecLab");
    let mut session = ReportSession::new(&mut store);
    // The session scrubs the log tail (and auto-adds the signed-in token) before it leaves.
    match session.submit(&transport, &draft, &meta, &logs, &[]) {
        Ok(issue) => {
            let _ = tx.send(ReportMsg::Submitted(issue));
        }
        Err(AuthError::Unauthorized) => {
            let _ = tx.send(ReportMsg::SubmitFailed {
                message: "your GitHub session expired".to_owned(),
                reauth: true,
            });
        }
        Err(e) => {
            let _ = tx.send(ReportMsg::SubmitFailed {
                message: e.to_string(),
                reauth: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dialog backed by an **in-memory** token store, so tests never touch the real OS
    /// keychain — which the CI build would otherwise use (the editor unifies to
    /// `assistant-net` there, so `ReportHost::new()` picks `KeychainTokenStore`).
    fn test_host() -> ReportHost {
        ReportHost::with_token_store(Box::new(seclab_ai::MemoryTokenStore::new()))
    }

    #[test]
    fn opens_and_closes() {
        let mut host = test_host();
        assert!(!host.open);
        host.open();
        assert!(host.open);
        assert_eq!(host.phase, Phase::Form);
    }

    #[test]
    fn reopening_after_done_starts_a_fresh_form() {
        let mut host = test_host();
        host.title = "old".to_owned();
        host.phase = Phase::Done;
        host.open();
        assert_eq!(host.phase, Phase::Form);
        assert!(host.title.is_empty(), "draft cleared on reopen after done");
    }

    #[test]
    fn code_message_shows_the_device_code_and_keeps_signing_in() {
        let mut host = test_host();
        host.apply_msg(ReportMsg::Code(DeviceCode {
            device_code: "dc".to_owned(),
            user_code: "WXYZ-1234".to_owned(),
            verification_uri: "https://github.com/login/device".to_owned(),
            expires_in: 900,
            interval: 5,
        }));
        assert_eq!(host.phase, Phase::SigningIn);
        assert_eq!(host.device.as_ref().unwrap().user_code, "WXYZ-1234");
    }

    #[test]
    fn signed_in_message_returns_to_the_form() {
        let mut host = test_host();
        host.phase = Phase::SigningIn;
        host.apply_msg(ReportMsg::SignedIn);
        assert_eq!(host.phase, Phase::Form);
        assert!(host.device.is_none());
        assert_eq!(host.status.as_deref(), Some("signed in to GitHub"));
    }

    #[test]
    fn submitted_message_shows_the_issue_link() {
        let mut host = test_host();
        host.phase = Phase::Submitting;
        host.apply_msg(ReportMsg::Submitted(CreatedIssue {
            number: 7,
            html_url: "https://github.com/ice1x/SecLab/issues/7".to_owned(),
        }));
        assert_eq!(host.phase, Phase::Done);
        assert_eq!(host.issue.as_ref().unwrap().number, 7);
    }

    #[test]
    fn a_401_submit_failure_reprompts_sign_in() {
        let mut host = test_host();
        host.phase = Phase::Submitting;
        host.apply_msg(ReportMsg::SubmitFailed {
            message: "your GitHub session expired".to_owned(),
            reauth: true,
        });
        assert_eq!(host.phase, Phase::Form);
        assert!(host.status.as_deref().unwrap().contains("sign in again"));
    }

    #[test]
    fn submit_requires_a_title() {
        let mut host = test_host();
        host.title = "   ".to_owned();
        host.begin_submit("abc1234");
        assert_eq!(host.status.as_deref(), Some("please enter a short title"));
        assert!(!host.in_flight(), "no worker spawned");
    }

    #[test]
    fn submit_requires_sign_in() {
        let mut host = test_host();
        host.title = "a real title".to_owned();
        // MemoryTokenStore starts signed out in the base build.
        host.begin_submit("abc1234");
        assert_eq!(host.status.as_deref(), Some("sign in to GitHub first"));
        assert!(!host.in_flight());
    }

    #[test]
    fn sign_out_clears_the_token() {
        let mut host = test_host();
        host.token.store("gho_tok").unwrap();
        assert!(host.signed_in());
        host.sign_out();
        assert!(!host.signed_in());
    }

    #[test]
    fn preview_scrubs_and_tags_the_title() {
        let mut host = test_host();
        host.title = "no sound".to_owned();
        host.description = "leaked gho_SECRETTOKEN here".to_owned();
        let preview = host.preview("deadbee");
        assert!(preview.contains("[Bug] no sound"));
        assert!(
            !preview.contains("gho_SECRETTOKEN"),
            "secret scrubbed in preview"
        );
        assert!(preview.contains("deadbee"), "build sha stamped");
    }

    #[test]
    fn preview_includes_the_shared_log_tail_scrubbed() {
        let mut host = test_host();
        let log = LogBuffer::new(10);
        log.push("boot ok");
        log.push("request used gho_LOGTOKEN then failed");
        host.set_log(log);
        host.title = "crash on load".to_owned();

        let preview = host.preview("sha1234");
        assert!(preview.contains("boot ok"), "log tail is attached");
        assert!(
            !preview.contains("gho_LOGTOKEN"),
            "a secret in the logs is scrubbed from the preview"
        );
    }
}
