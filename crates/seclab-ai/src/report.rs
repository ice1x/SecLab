//! The bug-report payload: what the "Report a problem / feature request" button sends,
//! and the privacy pass that runs before it leaves the machine (000252).
//!
//! A [`ReportDraft`] is what the user fills in (kind + title + description); [`Metadata`]
//! is the environment stamp collected automatically (app version, build commit, OS/arch).
//! [`build_issue`] turns a draft + metadata + a log excerpt into the [`IssuePayload`] that
//! the GitHub issue will carry — but **only after** [`scrub`] has removed every known
//! secret (the signed-in token, any BYOK API keys) and anything that merely looks like a
//! credential from the logs. [`IssuePayload::preview`] renders the exact text that would be
//! sent, so the app can show a "what will be sent?" screen and the user consents to the
//! real bytes — the same discipline the assistant uses for outgoing requests (see
//! [`crate::privacy`]).
//!
//! This module is pure logic with no IO and no network: collecting the screenshot and the
//! live log tail, and actually creating the issue, belong to the editor wiring (000253).

/// Whether the report is a defect or a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    /// Something is broken.
    Bug,
    /// A request for new behaviour.
    Feature,
}

impl ReportKind {
    /// The issue-title tag (`[Bug]` / `[Feature]`).
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            ReportKind::Bug => "[Bug]",
            ReportKind::Feature => "[Feature]",
        }
    }

    /// The human label shown in the issue body.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ReportKind::Bug => "Bug",
            ReportKind::Feature => "Feature request",
        }
    }
}

/// What the user types into the report form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportDraft {
    /// Bug vs. feature request.
    pub kind: ReportKind,
    /// A short one-line summary (becomes the issue title).
    pub title: String,
    /// The free-text description (becomes the issue body).
    pub description: String,
}

impl ReportDraft {
    /// A draft with the given kind, title and description.
    #[must_use]
    pub fn new(kind: ReportKind, title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            description: description.into(),
        }
    }

    /// Whether the draft has enough to submit: a non-blank title.
    #[must_use]
    pub fn is_submittable(&self) -> bool {
        !self.title.trim().is_empty()
    }
}

/// The automatically collected environment stamp attached to every report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// The app/crate version (`CARGO_PKG_VERSION`).
    pub version: String,
    /// The build commit SHA, when the build stamped one in.
    pub git_sha: Option<String>,
    /// The operating system (`std::env::consts::OS`).
    pub os: String,
    /// The CPU architecture (`std::env::consts::ARCH`).
    pub arch: String,
}

impl Metadata {
    /// The metadata for the current build/host: version from `CARGO_PKG_VERSION`, OS and
    /// arch from `std::env::consts`, and the optional build commit `git_sha`.
    #[must_use]
    pub fn current(git_sha: Option<String>) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            git_sha,
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }

    /// A one-line-per-field Markdown list for the issue body.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let sha = self.git_sha.as_deref().unwrap_or("unknown");
        format!(
            "- SecLab version: {}\n- Commit: {}\n- OS: {} {}",
            self.version, sha, self.os, self.arch
        )
    }
}

/// The placeholder a redacted secret is replaced with.
pub const REDACTION: &str = "«REDACTED»";

/// Token prefixes that mark a word as a credential even if it was not in the known list.
///
/// Covers GitHub tokens (`gho_`/`ghp_`/`ghu_`/`ghs_`/`ghr_`/`github_pat_`) and the common
/// LLM key shapes (`sk-ant-`, `sk-`), so a key pasted into a log by a dependency is caught
/// even when the app did not hand it to us.
const SECRET_PREFIXES: &[&str] = &[
    "gho_",
    "ghp_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "sk-ant-",
    "sk-",
];

/// Removes secrets from `text` before it can leave the machine.
///
/// Two passes: every non-empty value in `known` (the signed-in token, loaded API keys) is
/// replaced wherever it appears, then any remaining whitespace-delimited word that looks
/// like a credential (see [`SECRET_PREFIXES`]) is redacted. The result never contains a
/// value from `known`, so even a secret embedded mid-line is removed.
#[must_use]
pub fn scrub(text: &str, known: &[String]) -> String {
    // Pass 1: exact known secrets (longest first, so a token containing a shorter secret as
    // a substring is still fully removed rather than partially).
    let mut secrets: Vec<&str> = known
        .iter()
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .collect();
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut out = text.to_owned();
    for secret in secrets {
        if out.contains(secret) {
            out = out.replace(secret, REDACTION);
        }
    }

    // Pass 2: redact leftover credential-shaped words. Walk the text grouping maximal runs
    // of non-whitespace (words) and copying whitespace runs verbatim, so newlines, spaces
    // and tabs — the log's layout — are preserved and a token never spans a line break.
    let mut scrubbed = String::with_capacity(out.len());
    let mut word = String::new();
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !word.is_empty() {
                scrubbed.push_str(&redact_credential_word(&word));
                word.clear();
            }
            scrubbed.push(ch);
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        scrubbed.push_str(&redact_credential_word(&word));
    }
    scrubbed
}

/// Redacts a single whitespace-free word if it (after trimming surrounding punctuation)
/// begins with a known secret prefix; otherwise returns it unchanged.
fn redact_credential_word(word: &str) -> String {
    // A word may carry surrounding punctuation (e.g. `gho_abc,`); check the inner run of
    // token characters and redact only that, keeping the punctuation.
    let core = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
    if !core.is_empty() && SECRET_PREFIXES.iter().any(|p| core.starts_with(p)) {
        return word.replace(core, REDACTION);
    }
    word.to_owned()
}

/// The assembled issue a report becomes: a title and a Markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuePayload {
    /// The issue title (`[Bug] …` / `[Feature] …`).
    pub title: String,
    /// The Markdown issue body.
    pub body: String,
}

impl IssuePayload {
    /// The exact text that would be sent, for the "what will be sent?" review screen.
    #[must_use]
    pub fn preview(&self) -> String {
        format!("TITLE: {}\n\n{}", self.title, self.body)
    }
}

/// Builds the issue payload from a draft, the environment stamp and a log excerpt.
///
/// The `log_excerpt` (a tail of the session log) and the description are **scrubbed** with
/// `known_secrets` before assembly, so no secret the app holds — or any credential-shaped
/// string in the logs — reaches the body. An empty log excerpt simply omits the logs block.
#[must_use]
pub fn build_issue(
    draft: &ReportDraft,
    meta: &Metadata,
    log_excerpt: &str,
    known_secrets: &[String],
) -> IssuePayload {
    let title = format!("{} {}", draft.kind.tag(), draft.title.trim());

    let description = scrub(draft.description.trim(), known_secrets);
    let mut body = format!(
        "**Type:** {}\n\n**Description:**\n\n{}\n",
        draft.kind.label(),
        if description.is_empty() {
            "_(none provided)_"
        } else {
            &description
        }
    );

    body.push_str("\n<details><summary>Environment</summary>\n\n");
    body.push_str(&meta.to_markdown());
    body.push_str("\n\n</details>\n");

    let logs = scrub(log_excerpt.trim(), known_secrets);
    if !logs.is_empty() {
        body.push_str("\n<details><summary>Logs (tail, secrets scrubbed)</summary>\n\n```\n");
        body.push_str(&logs);
        body.push_str("\n```\n\n</details>\n");
    }

    IssuePayload { title, body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_needs_a_non_blank_title() {
        assert!(ReportDraft::new(ReportKind::Bug, "crash on load", "").is_submittable());
        assert!(!ReportDraft::new(ReportKind::Bug, "   ", "body").is_submittable());
    }

    #[test]
    fn metadata_current_reports_this_build() {
        let meta = Metadata::current(Some("abc1234".to_owned()));
        assert_eq!(meta.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(meta.os, std::env::consts::OS);
        assert!(meta.to_markdown().contains("abc1234"));
        // No SHA → "unknown", never a panic.
        assert!(Metadata::current(None).to_markdown().contains("unknown"));
    }

    #[test]
    fn scrub_removes_known_secrets_even_mid_line() {
        let secrets = vec!["gho_realtoken".to_owned(), "sk-ant-mykey".to_owned()];
        let text = "auth=gho_realtoken failed; key sk-ant-mykey rejected";
        let out = scrub(text, &secrets);
        assert!(!out.contains("gho_realtoken"));
        assert!(!out.contains("sk-ant-mykey"));
        assert_eq!(out.matches(REDACTION).count(), 2);
    }

    #[test]
    fn scrub_redacts_credential_shaped_words_not_in_the_known_list() {
        // A token the app never handed us, pasted into a log by something else.
        let out = scrub("leaked ghp_UNKNOWNSECRET here", &[]);
        assert!(!out.contains("ghp_UNKNOWNSECRET"), "unknown token redacted");
        assert!(out.contains("leaked"), "ordinary words kept");
        assert!(out.contains("here"));
    }

    #[test]
    fn scrub_handles_trailing_punctuation_on_a_token() {
        let out = scrub("token gho_abc123, done", &[]);
        assert!(!out.contains("gho_abc123"));
        assert!(out.contains(','), "surrounding punctuation preserved");
        assert!(out.contains("done"));
    }

    #[test]
    fn scrub_leaves_ordinary_text_untouched() {
        let text = "normal log line with no secrets at all";
        assert_eq!(scrub(text, &[]), text);
    }

    #[test]
    fn scrub_ignores_blank_known_entries() {
        // An empty known secret must not turn into a redact-everything bug.
        let out = scrub("hello world", &["".to_owned(), "   ".to_owned()]);
        assert_eq!(out, "hello world");
    }

    #[test]
    fn build_issue_tags_the_title_and_labels_the_type() {
        let draft = ReportDraft::new(ReportKind::Bug, "no sound after load", "VCA stuck closed");
        let meta = Metadata::current(Some("deadbee".to_owned()));
        let issue = build_issue(&draft, &meta, "", &[]);
        assert_eq!(issue.title, "[Bug] no sound after load");
        assert!(issue.body.contains("**Type:** Bug"));
        assert!(issue.body.contains("VCA stuck closed"));
        assert!(issue.body.contains("deadbee"), "env block present");
    }

    #[test]
    fn build_issue_scrubs_the_description_and_logs() {
        let draft = ReportDraft::new(
            ReportKind::Bug,
            "auth fails",
            "my token gho_SECRETTOKEN did not work",
        );
        let meta = Metadata::current(None);
        let logs = "line1\nBearer sk-ant-LEAKED\nline3";
        let issue = build_issue(&draft, &meta, logs, &["gho_SECRETTOKEN".to_owned()]);
        assert!(!issue.body.contains("gho_SECRETTOKEN"), "desc token gone");
        assert!(!issue.body.contains("sk-ant-LEAKED"), "log key gone");
        assert!(issue.body.contains("line1") && issue.body.contains("line3"));
        assert!(issue.body.contains("Logs (tail"), "logs block present");
    }

    #[test]
    fn build_issue_omits_the_logs_block_when_empty() {
        let draft = ReportDraft::new(ReportKind::Feature, "add reverb", "please");
        let issue = build_issue(&draft, &Metadata::current(None), "   ", &[]);
        assert!(
            !issue.body.contains("Logs"),
            "no logs block when excerpt blank"
        );
        assert!(issue.title.starts_with("[Feature]"));
    }

    #[test]
    fn empty_description_is_marked_not_left_blank() {
        let draft = ReportDraft::new(ReportKind::Feature, "idea", "");
        let issue = build_issue(&draft, &Metadata::current(None), "", &[]);
        assert!(issue.body.contains("_(none provided)_"));
    }

    #[test]
    fn preview_shows_title_and_body() {
        let draft = ReportDraft::new(ReportKind::Bug, "t", "d");
        let issue = build_issue(&draft, &Metadata::current(None), "", &[]);
        let preview = issue.preview();
        assert!(preview.starts_with("TITLE: [Bug] t"));
        assert!(preview.contains("**Description:**"));
    }
}
