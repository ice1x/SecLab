//! Build stamp: captures the short git SHA and the commit date at compile time so the editor
//! toolbar can show which build is running. The crate version is frozen at `0.0.1` pre-1.0 and
//! never moves between changes, so it cannot tell two builds apart — the git SHA does. Off a git
//! checkout (a packaged source tree, say) both fall back to `unknown`, and the label just reads
//! the plain version.
//!
//! No dependencies: it shells out to `git`. It re-runs when `HEAD` (or the ref log) changes, so the
//! stamp follows commits and branch switches without a manual clean.

use std::process::Command;

/// Runs `git` with `args` and returns its trimmed stdout, or `None` if git is missing, errors, or
/// prints nothing (e.g. not a checkout).
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn main() {
    // Short commit SHA, marked `+` when the working tree has uncommitted **source** changes — so a
    // locally tweaked build is never mistaken for the committed one. `Cargo.lock` is excluded from
    // the dirty check: a release build routinely re-resolves and rewrites it (an absolute-path dep
    // in this workspace churns it every build), so counting it would stamp every committed build
    // `+` and make the marker meaningless.
    let mut sha = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| {
        s.lines().any(|line| {
            // A porcelain line is "XY PATH" (PATH last, even for a rename's "old -> new"); the path
            // is the tail, so a change to anything but Cargo.lock leaves a line not ending in it.
            let line = line.trim();
            !line.is_empty() && !line.ends_with("Cargo.lock")
        })
    });
    if dirty {
        sha.push('+');
    }
    // The commit's date (YYYY-MM-DD) — stable across rebuilds of the same commit, unlike a build
    // clock, and enough to eyeball freshness.
    let date = git(&["show", "-s", "--format=%cd", "--date=short", "HEAD"])
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=VB_GIT_SHA={sha}");
    println!("cargo:rustc-env=VB_BUILD_DATE={date}");

    // Re-run when the checked-out commit moves (new commit, branch switch, checkout).
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/logs/HEAD");
}
