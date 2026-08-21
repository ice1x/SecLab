//! Code-signing and notarization helpers for the macOS bundles (backlog 000075).
//!
//! Distributing a plugin or app to another Mac requires an Apple **Developer ID**
//! signature and notarization, or Gatekeeper refuses to load it. The full manual process
//! is written up in [`docs/signing.md`](../../docs/signing.md); this module is the tooling
//! behind `cargo xtask sign` / `cargo xtask notarize`.
//!
//! The credential-dependent parts (a signing identity, a notary profile) cannot run in CI
//! or here, so the design keeps the fragile, order-sensitive pieces — the exact tool
//! argument vectors and which bundles to act on — as **pure, unit-tested functions**. The
//! subcommands in `main.rs` only supply the credentials from the environment and shell out;
//! when a credential is missing they fail with guidance instead of doing anything partial.

use std::path::{Path, PathBuf};

/// The bundle extensions we sign and notarize: the CLAP plus the three wrapped formats.
pub const SIGNABLE_EXTENSIONS: &[&str] = &["clap", "vst3", "component", "app"];

/// Arguments for `codesign` to apply a Developer ID signature with a hardened runtime and a
/// secure timestamp — the combination notarization requires.
///
/// `--deep` signs nested code (the wrapped bundles embed the CLAP); Apple discourages it
/// for complex apps in favour of inside-out signing, a caveat called out in the doc, but it
/// is the pragmatic choice for these self-contained plugin bundles.
#[must_use]
pub fn codesign_args(identity: &str, bundle: &Path) -> Vec<String> {
    vec![
        "--force".to_owned(),
        "--deep".to_owned(),
        "--options".to_owned(),
        "runtime".to_owned(),
        "--timestamp".to_owned(),
        "--sign".to_owned(),
        identity.to_owned(),
        bundle.display().to_string(),
    ]
}

/// Arguments for `codesign` to *verify* a signature (deep, strict, verbose).
#[must_use]
pub fn codesign_verify_args(bundle: &Path) -> Vec<String> {
    vec![
        "--verify".to_owned(),
        "--deep".to_owned(),
        "--strict".to_owned(),
        "--verbose=2".to_owned(),
        bundle.display().to_string(),
    ]
}

/// Arguments for `ditto` to zip a bundle for `notarytool`, which takes an archive
/// (`.zip`/`.dmg`/`.pkg`), never a raw `.app`/`.vst3` directory. `--keepParent` preserves
/// the bundle directory inside the archive.
#[must_use]
pub fn ditto_zip_args(bundle: &Path, archive: &Path) -> Vec<String> {
    vec![
        "-c".to_owned(),
        "-k".to_owned(),
        "--keepParent".to_owned(),
        bundle.display().to_string(),
        archive.display().to_string(),
    ]
}

/// Arguments for `xcrun notarytool submit … --wait` using a stored keychain profile
/// (created once with `xcrun notarytool store-credentials <profile>`).
#[must_use]
pub fn notarytool_submit_args(archive: &Path, keychain_profile: &str) -> Vec<String> {
    vec![
        "notarytool".to_owned(),
        "submit".to_owned(),
        archive.display().to_string(),
        "--keychain-profile".to_owned(),
        keychain_profile.to_owned(),
        "--wait".to_owned(),
    ]
}

/// Arguments for `xcrun stapler staple`, which attaches the notarization ticket to the
/// bundle so it validates offline.
#[must_use]
pub fn stapler_args(bundle: &Path) -> Vec<String> {
    vec![
        "stapler".to_owned(),
        "staple".to_owned(),
        bundle.display().to_string(),
    ]
}

/// The archive path for a bundle: the bundle path with `.zip` appended (e.g.
/// `vb-plugin.vst3` → `vb-plugin.vst3.zip`), so it sits next to the bundle.
#[must_use]
pub fn archive_path_for(bundle: &Path) -> PathBuf {
    let mut name = bundle.as_os_str().to_owned();
    name.push(".zip");
    PathBuf::from(name)
}

/// Lists the signable bundles directly inside `dir`, sorted for a stable order.
///
/// # Errors
/// Propagates any error from reading `dir` (e.g. it does not exist).
pub fn signable_bundles(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| SIGNABLE_EXTENSIONS.contains(&ext))
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn codesign_args_request_hardened_runtime_and_timestamp() {
        let args = codesign_args(
            "Developer ID Application: Acme (TEAMID)",
            Path::new("/b/x.vst3"),
        );
        // The signing options notarization checks for.
        assert!(args.contains(&"--options".to_owned()));
        assert!(args.contains(&"runtime".to_owned()));
        assert!(args.contains(&"--timestamp".to_owned()));
        assert!(args.contains(&"--force".to_owned()));
        // Identity is passed as the value after --sign, and the bundle is last.
        let sign_at = args.iter().position(|a| a == "--sign").unwrap();
        assert_eq!(args[sign_at + 1], "Developer ID Application: Acme (TEAMID)");
        assert_eq!(args.last().unwrap(), "/b/x.vst3");
    }

    #[test]
    fn ditto_zips_with_the_parent_directory_kept() {
        let args = ditto_zip_args(Path::new("/b/x.app"), Path::new("/b/x.app.zip"));
        assert_eq!(
            args,
            vec!["-c", "-k", "--keepParent", "/b/x.app", "/b/x.app.zip"]
        );
    }

    #[test]
    fn notarytool_submit_targets_the_archive_and_profile_and_waits() {
        let args = notarytool_submit_args(Path::new("/b/x.app.zip"), "vstbomb-notary");
        assert_eq!(args[0], "notarytool");
        assert_eq!(args[1], "submit");
        assert_eq!(args[2], "/b/x.app.zip");
        let p = args.iter().position(|a| a == "--keychain-profile").unwrap();
        assert_eq!(args[p + 1], "vstbomb-notary");
        assert!(
            args.contains(&"--wait".to_owned()),
            "must block on the result"
        );
    }

    #[test]
    fn stapler_staples_the_bundle() {
        assert_eq!(
            stapler_args(Path::new("/b/x.component")),
            vec!["stapler", "staple", "/b/x.component"]
        );
    }

    #[test]
    fn archive_path_appends_zip_next_to_the_bundle() {
        assert_eq!(
            archive_path_for(Path::new("/b/vb-plugin.vst3")),
            PathBuf::from("/b/vb-plugin.vst3.zip")
        );
    }

    #[test]
    fn signable_bundles_lists_only_known_extensions_sorted() {
        let dir = std::env::temp_dir().join(format!("vbsign-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["vb-plugin.vst3", "vb-plugin.clap", "notes.txt", "readme.md"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        // A plain file with a signable-looking extension is fine too — extension is the key.
        std::fs::write(dir.join("z.app"), b"").unwrap();

        let found = signable_bundles(&dir).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["vb-plugin.clap", "vb-plugin.vst3", "z.app"]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn signable_extensions_cover_the_four_formats() {
        for ext in ["clap", "vst3", "component", "app"] {
            assert!(SIGNABLE_EXTENSIONS.contains(&ext));
        }
    }
}
