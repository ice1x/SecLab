//! `cargo xtask` — build automation (task 000072).
//!
//! Two commands:
//!
//! - `cargo xtask bundle vb-plugin --release` — nih-plug's bundler: compiles the
//!   cdylib and wraps it into a loadable `.clap` bundle under `target/bundled/`.
//! - `cargo xtask export <patch.vbp>` — the one-command patch export: validates the
//!   project file (parse + migrate + full graph validation, so broken patches fail
//!   here and not inside a DAW), bundles the release plugin, and installs the patch
//!   into the bundle's `Contents/Resources/patch.vbp`. The plugin loads, in order:
//!   `VB_PATCH` env → the bundled resource patch → the built-in default.
//!
//! - `cargo xtask demo-patch <out.vbp>` — writes the built-in default patch as a
//!   starting-point project file.
//! - `cargo xtask validate` — bundles the release plugin and runs `clap-validator`
//!   over it (install once with
//!   `cargo install --git https://github.com/free-audio/clap-validator.git`).
//! - `cargo xtask bundle-vst3` — builds the VST3 via clap-wrapper (cloned and
//!   cached under `target/`; needs `cmake` and network on first run). The produced
//!   `vb-plugin.vst3` dynamically loads `vb-plugin.clap` from the standard CLAP
//!   locations, so install both: the `.clap` into `~/Library/Audio/Plug-Ins/CLAP`
//!   and the `.vst3` into `~/Library/Audio/Plug-Ins/VST3`.
//! - `cargo xtask bundle-auv2` — same, for the AUv2 `.component` (macOS; installs
//!   into `~/Library/Audio/Plug-Ins/Components`, validate with
//!   `auval -v aumu gWrp clAd`). Deterministically installs clap-wrapper's generated
//!   AudioComponents plist (its own install is flaky) and rebrands the manufacturer to
//!   `ice1x` so hosts list it as `ice1x: VST_bomb`.
//! - `cargo xtask bundle-standalone` — same, for a self-contained `vb-plugin.app`
//!   hosting the CLAP through RtAudio/RtMidi (the wrapper's simple runner; our own
//!   editor standalone lives in `vb-standalone`).
//! - `cargo xtask validate-au` — builds+installs the AUv2 and runs Apple's `auval` on it
//!   (reading its type/subtype/manufacturer from the Info.plist), the authoritative
//!   AudioUnit check. It bundles the AU itself, so it needs no prior `bundle-auv2`.
//! - `cargo xtask sign` — code-signs every bundle in `target/bundled/` with the Developer
//!   ID identity in `$DEVELOPER_ID` (hardened runtime + secure timestamp), then verifies.
//! - `cargo xtask notarize` — zips, submits to Apple's notary service via the
//!   `$NOTARY_PROFILE` keychain profile, and staples the ticket to each bundle. See
//!   [`docs/signing.md`](../docs/signing.md) for the full process and credential setup.
//!
//! Everything else is forwarded to nih-plug's xtask untouched.

use std::path::{Path, PathBuf};

mod sign;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).peekable();
    match args.peek().map(String::as_str) {
        Some("export") => {
            args.next();
            let patch = args.next().ok_or("usage: cargo xtask export <patch.vbp>")?;
            export(Path::new(&patch))
        }
        Some("validate") => {
            args.next();
            validate()
        }
        Some("bundle-vst3") => {
            args.next();
            bundle_wrapped(WrappedFormat::Vst3)
        }
        Some("bundle-auv2") => {
            args.next();
            bundle_wrapped(WrappedFormat::Auv2)
        }
        Some("bundle-standalone") => {
            args.next();
            bundle_wrapped(WrappedFormat::Standalone)
        }
        Some("sign") => {
            args.next();
            sign_bundles()
        }
        Some("notarize") => {
            args.next();
            notarize_bundles()
        }
        Some("validate-au") => {
            args.next();
            validate_au()
        }
        Some("demo-patch") => {
            args.next();
            let out = args
                .next()
                .ok_or("usage: cargo xtask demo-patch <out.vbp>")?;
            let doc = vb_plugin::default_patch(&vb_nodes::Registry::with_builtins());
            vb_project::save_to_path(&doc, Path::new(&out))?;
            println!("wrote {out}");
            Ok(())
        }
        _ => nih_plug_xtask::main_with_args("cargo xtask", args).map_err(Into::into),
    }
}

/// Known-upstream exclusions, each with a reason:
///
/// - `state-reproducibility-*`: nih-plug's CLAP wrapper applies host-initiated
///   state loads correctly but never requests `CLAP_PARAM_RESCAN_VALUES`
///   afterwards (only the GUI path does), so the validator flags the value
///   change as "changed without a rescan request". Wrapper-level; revisit on
///   nih-plug updates.
/// - `state-invalid-random`: the validator's own harness aborts on the random
///   state it generates (absurd allocation) — reproducible with a plain
///   nih-plug plugin.
const VALIDATOR_EXCLUDES: &[&str] = &["state-reproducibility", "state-invalid-random"];

/// Bundles the release plugin and runs clap-validator over the bundle.
fn validate() -> Result<(), Box<dyn std::error::Error>> {
    nih_plug_xtask::main_with_args(
        "cargo xtask",
        ["bundle", "vb-plugin", "--release"]
            .into_iter()
            .map(String::from),
    )?;
    let bundle = bundle_dir()?;

    let mut cmd = std::process::Command::new("clap-validator");
    cmd.arg("validate");
    for exclude in VALIDATOR_EXCLUDES {
        cmd.args(["--exclude", exclude]);
    }
    cmd.arg(&bundle);
    let status = cmd.status().map_err(|e| {
        format!(
            "failed to run clap-validator ({e}); install it with \
             `cargo install --git https://github.com/free-audio/clap-validator.git`"
        )
    })?;
    if !status.success() {
        return Err(format!("clap-validator failed on {}", bundle.display()).into());
    }
    println!("clap-validator passed: {}", bundle.display());
    Ok(())
}

/// The clap-wrapper revision we build against (pinned for reproducibility).
const CLAP_WRAPPER_REPO: &str = "https://github.com/free-audio/clap-wrapper.git";

/// Which clap-wrapper output to build.
#[derive(Clone, Copy)]
enum WrappedFormat {
    Vst3,
    Auv2,
    Standalone,
}

impl WrappedFormat {
    fn build_dir(self) -> &'static str {
        match self {
            Self::Vst3 => "build-vst3",
            Self::Auv2 => "build-auv2",
            Self::Standalone => "build-standalone",
        }
    }

    fn artifact(self) -> &'static str {
        match self {
            Self::Vst3 => "vb-plugin.vst3",
            Self::Auv2 => "vb-plugin.component",
            Self::Standalone => "vb-plugin.app",
        }
    }

    fn install_dir(self) -> &'static str {
        match self {
            Self::Vst3 => "~/Library/Audio/Plug-Ins/VST3/",
            Self::Auv2 => "~/Library/Audio/Plug-Ins/Components/",
            Self::Standalone => "/Applications/",
        }
    }

    fn extra_cmake_flag(self) -> Option<&'static str> {
        match self {
            Self::Vst3 => None,
            Self::Auv2 => Some("-DCLAP_WRAPPER_BUILD_AUV2=ON"),
            Self::Standalone => Some("-DCLAP_WRAPPER_BUILD_STANDALONE=ON"),
        }
    }
}

/// Builds the release CLAP, then wraps it into the requested format via clap-wrapper.
fn bundle_wrapped(format: WrappedFormat) -> Result<(), Box<dyn std::error::Error>> {
    // The wrapped bundles (VST3/AU/standalone) ship with the embedded editor (000067) —
    // `--features gui`. Plain `validate()` deliberately does NOT, so the CI clap-validator
    // never opens a window on a headless runner.
    nih_plug_xtask::main_with_args(
        "cargo xtask",
        ["bundle", "vb-plugin", "--release", "--features", "gui"]
            .into_iter()
            .map(String::from),
    )?;
    let root = workspace_root()?;
    let clap_bundle = bundle_dir()?;

    // Clone once, reuse forever (delete target/clap-wrapper to refresh).
    let wrapper_src = root.join("target").join("clap-wrapper");
    if !wrapper_src.exists() {
        run(
            std::process::Command::new("git")
                .args(["clone", "--depth", "1", CLAP_WRAPPER_REPO])
                .arg(&wrapper_src),
            "git clone clap-wrapper",
        )?;
    }

    let build_dir = wrapper_src.join(format.build_dir());
    let mut configure = std::process::Command::new("cmake");
    configure
        .arg("-S")
        .arg(&wrapper_src)
        .arg("-B")
        .arg(&build_dir)
        .args([
            "-DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=ON",
            "-DCLAP_WRAPPER_OUTPUT_NAME=vb-plugin",
            "-DCMAKE_BUILD_TYPE=Release",
            // Match the Rust cdylib's architecture: an x86_64 cmake (Rosetta)
            // would otherwise build a wrapper that cannot dlopen our clap.
            #[cfg(target_arch = "aarch64")]
            "-DCMAKE_OSX_ARCHITECTURES=arm64",
            #[cfg(target_arch = "x86_64")]
            "-DCMAKE_OSX_ARCHITECTURES=x86_64",
        ]);
    if let Some(flag) = format.extra_cmake_flag() {
        configure.arg(flag);
    }
    run(
        &mut configure,
        "cmake configure (needs network on first run)",
    )?;
    run(
        std::process::Command::new("cmake")
            .arg("--build")
            .arg(&build_dir)
            .args(["--config", "Release", "-j8"]),
        "cmake build",
    )?;

    // Ship the wrapper next to the clap it loads.
    let produced = build_dir.join(format.artifact());
    let dest = root.join("target").join("bundled").join(format.artifact());
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    copy_tree(&produced, &dest)?;

    // Fix + brand the AU Info.plist. clap-wrapper GENERATES a correct AudioComponents plist
    // (`…-build-helper-output/auv2_Info.plist`) but installs it into the `.component` only
    // flakily — some rebuilds leave a default `APPL` plist with no AudioComponents, so the
    // AU won't register (auval "didn't find the component"). Install the generated plist
    // deterministically, then rebrand its "Manufacturer: Plugin" display name (clap-wrapper
    // hardcodes "cleveraudio.org" in top_level_default.cmake, with no cache-var override).
    if matches!(format, WrappedFormat::Auv2) {
        let dest_plist = dest.join("Contents").join("Info.plist");
        let generated = find_generated_au_plist(&build_dir).ok_or(
            "could not find clap-wrapper's generated auv2_Info.plist to install; \
             the AU would not register",
        )?;
        std::fs::copy(&generated, &dest_plist)?;
        run(
            std::process::Command::new("/usr/libexec/PlistBuddy")
                .args(au_set_name_args(&dest_plist, AU_DISPLAY_NAME)),
            "brand AU component display name",
        )?;
        println!("installed + branded AU plist as: {AU_DISPLAY_NAME}");
    }

    println!("built: {}", dest.display());
    println!(
        "install: cp -r {} ~/Library/Audio/Plug-Ins/CLAP/ && cp -r {} {}",
        clap_bundle.display(),
        dest.display(),
        format.install_dir()
    );
    Ok(())
}

/// The AU "Manufacturer: Plugin" display name hosts (Logic, GarageBand) group the component
/// under. Mirrors vb-plugin's `VENDOR` / `NAME`; overrides clap-wrapper's hardcoded
/// "cleveraudio.org: <output>" default (see `bundle_wrapped`).
const AU_DISPLAY_NAME: &str = "ice1x: VST_bomb";

/// Locates clap-wrapper's generated AU `Info.plist` under the AUv2 build dir — the file in
/// a `*-build-helper-output/` subdirectory named `auv2_Info.plist`. This is the
/// reliably-correct AudioComponents plist to install into the bundle (clap-wrapper's own
/// install of it is flaky across rebuilds).
fn find_generated_au_plist(build_dir: &std::path::Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(build_dir).ok()?.flatten() {
        let dir = entry.path();
        let is_helper = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("-build-helper-output"));
        if dir.is_dir() && is_helper {
            let plist = dir.join("auv2_Info.plist");
            if plist.exists() {
                return Some(plist);
            }
        }
    }
    None
}

/// `PlistBuddy` args to set an AU component's AudioComponents display name.
///
/// PlistBuddy takes the whole `-c` string as one command, with everything after the key
/// path as the (possibly space-containing) value — so the name needs no quoting here.
fn au_set_name_args(plist: &std::path::Path, name: &str) -> Vec<String> {
    vec![
        "-c".to_owned(),
        format!("Set :AudioComponents:0:name {name}"),
        plist.display().to_string(),
    ]
}

/// Runs a command, turning non-zero exits into errors with context.
fn run(cmd: &mut std::process::Command, what: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if !status.success() {
        return Err(format!("{what} failed with {status}").into());
    }
    Ok(())
}

/// Arguments for Apple's `auval` to validate a single AudioUnit by its component codes
/// (`auval -v <type> <subtype> <manufacturer>`).
#[must_use]
pub fn auval_args(kind: &str, subtype: &str, manufacturer: &str) -> Vec<String> {
    vec![
        "-v".to_owned(),
        kind.to_owned(),
        subtype.to_owned(),
        manufacturer.to_owned(),
    ]
}

/// Reads one `AudioComponents:0:<key>` value from an AU `Info.plist` via `PlistBuddy`.
fn au_code(plist: &Path, key: &str) -> Result<String, Box<dyn std::error::Error>> {
    let out = std::process::Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(format!("Print :AudioComponents:0:{key}"))
        .arg(plist)
        .output()
        .map_err(|e| format!("PlistBuddy read {key}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "could not read AudioComponents:0:{key} from {}",
            plist.display()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Builds the AUv2 component and validates it with Apple's `auval`.
///
/// It bundles+installs the AU itself (rather than relying on a prior `bundle-auv2` step) so
/// the build location and the validation location are resolved by the *same* `workspace_root`
/// call in one process — in CI the runner checkout is nested inside the developer clone, and
/// splitting the two across steps let them disagree on which `target/bundled` to use.
///
/// Reads the component's type/subtype/manufacturer codes from its `Info.plist` and runs
/// `auval -v` on them — the authoritative macOS AudioUnit check (format negotiation, render
/// at many sample rates, parameters, MIDI).
///
/// # Errors
///
/// Returns an error if the AU cannot be built, its codes cannot be read, or `auval` fails.
fn validate_au() -> Result<(), Box<dyn std::error::Error>> {
    // Build + install the AU first, in this same process (see the note above).
    bundle_wrapped(WrappedFormat::Auv2)?;

    let component = workspace_root()?
        .join("target")
        .join("bundled")
        .join("vb-plugin.component");
    let plist = component.join("Contents").join("Info.plist");
    if !plist.exists() {
        return Err(format!(
            "no AU component at {} after bundling — the clap-wrapper build may have failed",
            component.display()
        )
        .into());
    }
    let kind = au_code(&plist, "type")?;
    let subtype = au_code(&plist, "subtype")?;
    let manufacturer = au_code(&plist, "manufacturer")?;
    println!("validating AU ({kind} {subtype} {manufacturer}) with auval…");
    let mut cmd = std::process::Command::new("auval");
    cmd.args(auval_args(&kind, &subtype, &manufacturer));
    run(&mut cmd, "auval").map_err(|e| {
        format!("{e}; ensure the AU is installed (bundle-auv2 installs it) and auval is on PATH")
            .into()
    })
}

/// Recursively copies a bundle directory.
fn copy_tree(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// Validates `patch`, bundles the release plugin, installs the patch as a resource.
fn export(patch: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Fail early on anything the engine would refuse later.
    let doc = vb_project::open_path(patch)
        .map_err(|e| format!("patch '{}' is not exportable: {e}", patch.display()))?;
    println!(
        "patch ok: {} nodes, {} edges",
        doc.nodes.len(),
        doc.edges.len()
    );

    // 2. Build the release bundle via nih-plug's bundler.
    nih_plug_xtask::main_with_args(
        "cargo xtask",
        ["bundle", "vb-plugin", "--release"]
            .into_iter()
            .map(String::from),
    )?;

    // 3. Install the patch inside the bundle so it travels with the plugin.
    let bundle = bundle_dir()?;
    let resources = bundle.join("Contents").join("Resources");
    std::fs::create_dir_all(&resources)?;
    let dest = resources.join("patch.vbp");
    std::fs::copy(patch, &dest)?;
    println!("exported: {} (patch embedded)", bundle.display());
    Ok(())
}

/// The `target/bundled/` directory holding the produced bundles.
fn bundled_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(workspace_root()?.join("target").join("bundled"))
}

/// Code-signs every bundle in `target/bundled/` with a Developer ID identity.
///
/// The identity comes from `$DEVELOPER_ID` (e.g.
/// `"Developer ID Application: Your Name (TEAMID)"`); without it, nothing is signed and the
/// command explains what to set. See `docs/signing.md` for the full flow.
fn sign_bundles() -> Result<(), Box<dyn std::error::Error>> {
    let identity = std::env::var("DEVELOPER_ID").map_err(|_| {
        "set DEVELOPER_ID to your signing identity, e.g. \
         DEVELOPER_ID='Developer ID Application: Your Name (TEAMID)' \
         (list identities with `security find-identity -v -p codesigning`); see docs/signing.md"
    })?;

    let dir = bundled_dir()?;
    let bundles = sign::signable_bundles(&dir).map_err(|e| {
        format!(
            "no bundles to sign in {} ({e}); build them first with `cargo xtask bundle-vst3` etc.",
            dir.display()
        )
    })?;
    if bundles.is_empty() {
        return Err(format!(
            "no signable bundles in {}; build them first (bundle-vst3 / bundle-auv2 / bundle-standalone)",
            dir.display()
        )
        .into());
    }

    for bundle in &bundles {
        run(
            std::process::Command::new("codesign").args(sign::codesign_args(&identity, bundle)),
            &format!("codesign {}", bundle.display()),
        )?;
        run(
            std::process::Command::new("codesign").args(sign::codesign_verify_args(bundle)),
            &format!("verify signature of {}", bundle.display()),
        )?;
        println!("signed: {}", bundle.display());
    }
    println!(
        "signed {} bundle(s). Next: `cargo xtask notarize`.",
        bundles.len()
    );
    Ok(())
}

/// Notarizes every bundle in `target/bundled/`: zip → `notarytool submit --wait` → staple.
///
/// The notary credentials come from a stored keychain profile named by `$NOTARY_PROFILE`
/// (create it once with `xcrun notarytool store-credentials`). Bundles must already be
/// signed with a hardened runtime (`cargo xtask sign`). See `docs/signing.md`.
fn notarize_bundles() -> Result<(), Box<dyn std::error::Error>> {
    let profile = std::env::var("NOTARY_PROFILE").map_err(|_| {
        "set NOTARY_PROFILE to a stored notarytool keychain profile \
         (create it once with `xcrun notarytool store-credentials <profile> \
         --apple-id <id> --team-id <TEAMID>`); see docs/signing.md"
    })?;

    let dir = bundled_dir()?;
    let bundles = sign::signable_bundles(&dir)
        .map_err(|e| format!("no bundles to notarize in {} ({e})", dir.display()))?;
    if bundles.is_empty() {
        return Err(format!(
            "no bundles to notarize in {}; sign them first",
            dir.display()
        )
        .into());
    }

    for bundle in &bundles {
        let archive = sign::archive_path_for(bundle);
        if archive.exists() {
            std::fs::remove_file(&archive)?;
        }
        run(
            std::process::Command::new("ditto").args(sign::ditto_zip_args(bundle, &archive)),
            &format!("zip {} for notarization", bundle.display()),
        )?;
        run(
            std::process::Command::new("xcrun")
                .args(sign::notarytool_submit_args(&archive, &profile)),
            &format!("notarize {}", archive.display()),
        )?;
        run(
            std::process::Command::new("xcrun").args(sign::stapler_args(bundle)),
            &format!("staple {}", bundle.display()),
        )?;
        std::fs::remove_file(&archive).ok();
        println!("notarized + stapled: {}", bundle.display());
    }
    println!("notarized {} bundle(s).", bundles.len());
    Ok(())
}

/// Locates the produced `.clap` bundle under `target/bundled/`.
fn bundle_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let bundled = workspace_root()?.join("target").join("bundled");
    let entry = std::fs::read_dir(&bundled)?
        .filter_map(Result::ok)
        .find(|e| e.path().extension().is_some_and(|ext| ext == "clap"))
        .ok_or_else(|| format!("no .clap bundle found in {}", bundled.display()))?;
    Ok(entry.path())
}

/// The workspace root (xtask runs from anywhere inside it).
fn workspace_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("Cargo.toml").exists()
            && std::fs::read_to_string(dir.join("Cargo.toml"))?.contains("[workspace]")
        {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err("workspace root not found".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{au_set_name_args, auval_args, WrappedFormat, AU_DISPLAY_NAME};
    use std::path::Path;

    #[test]
    fn auval_args_pass_the_component_codes_after_v() {
        // auval -v <type> <subtype> <manufacturer> — the order auval expects.
        assert_eq!(
            auval_args("aumu", "gWrp", "clAd"),
            vec!["-v", "aumu", "gWrp", "clAd"]
        );
    }

    #[test]
    fn au_name_command_sets_the_first_audiocomponents_name() {
        let args = au_set_name_args(
            Path::new("/b/vb-plugin.component/Contents/Info.plist"),
            AU_DISPLAY_NAME,
        );
        assert_eq!(args[0], "-c");
        // Key path then the raw value (spaces and the colon in "ice1x: VST_bomb" are fine —
        // PlistBuddy takes the rest of the -c string verbatim as the value).
        assert_eq!(args[1], "Set :AudioComponents:0:name ice1x: VST_bomb");
        assert_eq!(args[2], "/b/vb-plugin.component/Contents/Info.plist");
    }

    #[test]
    fn finds_the_generated_au_plist_in_a_build_helper_output_dir() {
        use super::find_generated_au_plist;
        let root = std::env::temp_dir().join(format!("vbau-{}", std::process::id()));
        let helper = root.join("vb_plugin_as_auv2-build-helper-output");
        std::fs::create_dir_all(&helper).unwrap();
        std::fs::write(helper.join("auv2_Info.plist"), b"<plist/>").unwrap();
        // A decoy dir that is not the helper output.
        std::fs::create_dir_all(root.join("cpm")).unwrap();

        let found = find_generated_au_plist(&root).expect("generated plist located");
        assert_eq!(found, helper.join("auv2_Info.plist"));

        // Absent → None.
        std::fs::remove_file(helper.join("auv2_Info.plist")).unwrap();
        assert!(find_generated_au_plist(&root).is_none());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn au_display_name_is_manufacturer_then_plugin() {
        // Hosts split on ": " → manufacturer "ice1x", plugin "VST_bomb".
        let (manufacturer, plugin) = AU_DISPLAY_NAME
            .split_once(": ")
            .expect("Manufacturer: Plugin");
        assert_eq!(manufacturer, "ice1x");
        assert_eq!(plugin, "VST_bomb");
    }

    /// The real-host testing checklist must name every bundle the build produces, so the
    /// install table there cannot silently drift from what `bundle-*` actually emits.
    #[test]
    fn host_testing_checklist_names_every_produced_bundle() {
        let doc = include_str!("../../docs/host-testing.md");
        // The CLAP is produced by the plain bundler; the other three by the wrapper.
        let mut artifacts = vec!["vb-plugin.clap"];
        for format in [
            WrappedFormat::Vst3,
            WrappedFormat::Auv2,
            WrappedFormat::Standalone,
        ] {
            artifacts.push(format.artifact());
        }
        for artifact in artifacts {
            assert!(
                doc.contains(artifact),
                "docs/host-testing.md must mention the produced bundle `{artifact}`"
            );
        }
    }
}
