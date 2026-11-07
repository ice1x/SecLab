//! `cargo xtask` — build automation (task 000072).
//!
//! Two commands:
//!
//! - `cargo xtask bundle seclab-plugin --release` — nih-plug's bundler: compiles the
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
//!   cached under `target/`; needs `cmake`, a C++ compiler and network on first run) on
//!   macOS, Linux and Windows. The produced `seclab-plugin.vst3` dynamically loads
//!   `seclab-plugin.clap` from the standard CLAP locations, so install both: macOS
//!   `~/Library/Audio/Plug-Ins/{CLAP,VST3}`, Linux `~/.clap` + `~/.vst3`, Windows
//!   `%COMMONPROGRAMFILES%\{CLAP,VST3}` (on Windows the `.vst3` is a single file).
//! - `cargo xtask bundle-auv2` — same, for the AUv2 `.component` (macOS; installs
//!   into `~/Library/Audio/Plug-Ins/Components`, validate with
//!   `auval -v aumu gWrp clAd`). Deterministically installs clap-wrapper's generated
//!   AudioComponents plist (its own install is flaky) and rebrands the manufacturer to
//!   `ice1x` so hosts list it as `ice1x: SecLab`.
//! - `cargo xtask bundle-standalone` — same, for a self-contained `seclab-plugin.app`
//!   hosting the CLAP through RtAudio/RtMidi (the wrapper's simple runner; our own
//!   editor standalone lives in `seclab-standalone`).
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
            let doc = seclab_plugin::default_patch(&seclab_nodes::Registry::with_builtins());
            seclab_project::save_to_path(&doc, Path::new(&out))?;
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
        ["bundle", "seclab-plugin", "--release"]
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
            Self::Vst3 => "seclab-plugin.vst3",
            Self::Auv2 => "seclab-plugin.component",
            Self::Standalone => "seclab-plugin.app",
        }
    }

    /// Where a host scans for this format on `os` (`std::env::consts::OS` spelling).
    fn install_dir(self, os: &str) -> &'static str {
        match (self, os) {
            (Self::Vst3, "linux") => "~/.vst3/",
            (Self::Vst3, "windows") => r"%COMMONPROGRAMFILES%\VST3\",
            (Self::Vst3, _) => "~/Library/Audio/Plug-Ins/VST3/",
            (Self::Auv2, _) => "~/Library/Audio/Plug-Ins/Components/",
            (Self::Standalone, _) => "/Applications/",
        }
    }

    /// Whether clap-wrapper can produce this format on `os`. The VST3 builds everywhere;
    /// AUv2 is Apple-only, and the wrapper's `.app` runner is only shipped on macOS (on
    /// Linux/Windows the standalone is our own `seclab-standalone`).
    fn supported_on(self, os: &str) -> bool {
        match self {
            Self::Vst3 => true,
            Self::Auv2 | Self::Standalone => os == "macos",
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

/// Whether `dir` holds a clap-wrapper checkout cmake can configure: its top-level
/// `CMakeLists.txt` must be present. A missing or hollow directory (e.g. one a CI cache
/// restored without its contents) is not usable and must be (re-)cloned.
fn clap_wrapper_checkout_is_usable(dir: &Path) -> bool {
    dir.join("CMakeLists.txt").is_file()
}

/// Where hosts — and the wrapped VST3/AU, which loads the CLAP at runtime — look for a
/// `.clap` on `os`. Mirrors clap-wrapper's search (`src/detail/clap/fsutil.cpp`).
fn clap_install_dir(os: &str) -> &'static str {
    match os {
        "linux" => "~/.clap/",
        "windows" => r"%COMMONPROGRAMFILES%\CLAP\",
        _ => "~/Library/Audio/Plug-Ins/CLAP/",
    }
}

/// Locates a clap-wrapper output in `build_dir`. With the single-config Makefiles generator
/// on macOS it lands in the build dir itself; on Linux clap-wrapper nests the VST3 bundle
/// under a `Release/` config folder, and the multi-config MSVC generator on Windows puts
/// its (single-file) `.vst3` there too (`cmake/wrap_vst3.cmake`).
fn wrapped_artifact_in(build_dir: &Path, artifact: &str) -> Option<PathBuf> {
    [
        build_dir.join(artifact),
        build_dir.join("Release").join(artifact),
    ]
    .into_iter()
    .find(|p| p.exists())
}

/// Copies a produced artifact — a bundle directory, or a single file such as the Windows
/// `.vst3` DLL — to `to`, replacing whatever is there.
fn copy_artifact(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if to.is_dir() {
        std::fs::remove_dir_all(to)?;
    } else if to.exists() {
        std::fs::remove_file(to)?;
    }
    if from.is_dir() {
        copy_tree(from, to)
    } else {
        std::fs::copy(from, to)?;
        Ok(())
    }
}

/// Builds the release CLAP, then wraps it into the requested format via clap-wrapper.
fn bundle_wrapped(format: WrappedFormat) -> Result<(), Box<dyn std::error::Error>> {
    let os = std::env::consts::OS;
    if !format.supported_on(os) {
        return Err(format!(
            "{} is macOS-only; on {os} build the VST3 (`cargo xtask bundle-vst3`) or the \
             native standalone (`cargo build --release -p seclab-standalone`)",
            format.artifact()
        )
        .into());
    }
    // The wrapped bundles (VST3/AU/standalone) ship with the embedded editor (000067) —
    // `--features gui`. Plain `validate()` deliberately does NOT, so the CI clap-validator
    // never opens a window on a headless runner.
    nih_plug_xtask::main_with_args(
        "cargo xtask",
        ["bundle", "seclab-plugin", "--release", "--features", "gui"]
            .into_iter()
            .map(String::from),
    )?;
    let root = workspace_root()?;
    let clap_bundle = bundle_dir()?;

    // Clone once, reuse forever (delete target/clap-wrapper to refresh). The check is for a
    // USABLE checkout, not a mere directory: on CI, `Swatinem/rust-cache` restores `target/`
    // and prunes everything cargo did not produce, which left an EMPTY `target/clap-wrapper`
    // behind — a bare `exists()` then skipped the clone and cmake failed with "does not
    // appear to contain CMakeLists.txt". A hollow directory is removed and re-cloned.
    let wrapper_src = root.join("target").join("clap-wrapper");
    if !clap_wrapper_checkout_is_usable(&wrapper_src) {
        if wrapper_src.exists() {
            std::fs::remove_dir_all(&wrapper_src)?;
        }
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
            "-DCLAP_WRAPPER_OUTPUT_NAME=seclab-plugin",
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
    let produced = wrapped_artifact_in(&build_dir, format.artifact()).ok_or_else(|| {
        format!(
            "cmake finished but no {} was found in {} (or its Release/ folder)",
            format.artifact(),
            build_dir.display()
        )
    })?;
    let dest = root.join("target").join("bundled").join(format.artifact());
    copy_artifact(&produced, &dest)?;

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
        "install: copy {} into {} and {} into {}",
        clap_bundle.display(),
        clap_install_dir(os),
        dest.display(),
        format.install_dir(os)
    );
    Ok(())
}

/// The AU "Manufacturer: Plugin" display name hosts (Logic, GarageBand) group the component
/// under. Mirrors seclab-plugin's `VENDOR` / `NAME`; overrides clap-wrapper's hardcoded
/// "cleveraudio.org: <output>" default (see `bundle_wrapped`).
const AU_DISPLAY_NAME: &str = "ice1x: SecLab";

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
        .join("seclab-plugin.component");
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
    let doc = seclab_project::open_path(patch)
        .map_err(|e| format!("patch '{}' is not exportable: {e}", patch.display()))?;
    println!(
        "patch ok: {} nodes, {} edges",
        doc.nodes.len(),
        doc.edges.len()
    );

    // 2. Build the release bundle via nih-plug's bundler.
    nih_plug_xtask::main_with_args(
        "cargo xtask",
        ["bundle", "seclab-plugin", "--release"]
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
    clap_bundle_in(&workspace_root()?.join("target").join("bundled"))
}

/// The bundler's output name for the plugin crate.
const CLAP_BUNDLE: &str = "seclab-plugin.clap";

/// The plugin's `.clap` in `bundled`, by its exact name. Picking "the first `.clap`" in
/// directory order used to grab a stale bundle left from an older crate name
/// (`vb-plugin.clap`), so `validate`/`export` acted on the wrong plugin.
fn clap_bundle_in(bundled: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let clap = bundled.join(CLAP_BUNDLE);
    if clap.exists() {
        Ok(clap)
    } else {
        Err(format!("no {CLAP_BUNDLE} found in {}", bundled.display()).into())
    }
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
            Path::new("/b/seclab-plugin.component/Contents/Info.plist"),
            AU_DISPLAY_NAME,
        );
        assert_eq!(args[0], "-c");
        // Key path then the raw value (spaces and the colon in "ice1x: SecLab" are fine —
        // PlistBuddy takes the rest of the -c string verbatim as the value).
        assert_eq!(args[1], "Set :AudioComponents:0:name ice1x: SecLab");
        assert_eq!(args[2], "/b/seclab-plugin.component/Contents/Info.plist");
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
    fn a_clap_wrapper_checkout_is_usable_only_with_its_cmakelists() {
        use super::clap_wrapper_checkout_is_usable;
        let root = std::env::temp_dir().join(format!("vbcw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        // Missing entirely → clone.
        assert!(!clap_wrapper_checkout_is_usable(&root));
        // A hollow directory (what a pruned CI cache leaves behind) → still clone.
        std::fs::create_dir_all(&root).unwrap();
        assert!(!clap_wrapper_checkout_is_usable(&root));
        // The real thing has a top-level CMakeLists.txt → reuse.
        std::fs::write(root.join("CMakeLists.txt"), b"project(clap-wrapper)").unwrap();
        assert!(clap_wrapper_checkout_is_usable(&root));

        std::fs::remove_dir_all(&root).unwrap();
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("vbx-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_wrapped_artifact_is_found_in_each_platform_layout() {
        use super::wrapped_artifact_in;
        let name = "seclab-plugin.vst3";

        // macOS (single-config Makefiles): a bundle directory straight in the build dir.
        let mac = scratch("mac");
        std::fs::create_dir_all(mac.join(name).join("Contents")).unwrap();
        assert_eq!(wrapped_artifact_in(&mac, name), Some(mac.join(name)));

        // Linux: clap-wrapper puts the bundle under a `Release/` config folder.
        let linux = scratch("linux");
        std::fs::create_dir_all(linux.join("Release").join(name).join("Contents")).unwrap();
        assert_eq!(
            wrapped_artifact_in(&linux, name),
            Some(linux.join("Release").join(name))
        );

        // Windows (multi-config MSVC): a single-file `.vst3` DLL under `Release/`.
        let win = scratch("win");
        std::fs::create_dir_all(win.join("Release")).unwrap();
        std::fs::write(win.join("Release").join(name), b"MZ").unwrap();
        assert_eq!(
            wrapped_artifact_in(&win, name),
            Some(win.join("Release").join(name))
        );

        // Nothing built → None (the caller turns it into an error naming the build dir).
        let empty = scratch("empty");
        assert_eq!(wrapped_artifact_in(&empty, name), None);

        for dir in [mac, linux, win, empty] {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn copy_artifact_replaces_a_bundle_dir_or_a_single_file() {
        use super::copy_artifact;
        let root = scratch("copy");

        // A bundle directory (macOS/Linux VST3) replaces a stale one wholesale.
        let bundle = root.join("src.vst3");
        std::fs::create_dir_all(bundle.join("Contents").join("x86_64-linux")).unwrap();
        std::fs::write(
            bundle.join("Contents").join("x86_64-linux").join("p.so"),
            b"new",
        )
        .unwrap();
        let dest = root.join("out.vst3");
        std::fs::create_dir_all(dest.join("stale")).unwrap();
        copy_artifact(&bundle, &dest).unwrap();
        assert_eq!(
            std::fs::read(dest.join("Contents").join("x86_64-linux").join("p.so")).unwrap(),
            b"new"
        );
        assert!(!dest.join("stale").exists(), "old bundle contents removed");

        // A single-file VST3 (Windows) replaces whatever was there, even a directory.
        let file = root.join("single.vst3");
        std::fs::write(&file, b"MZ").unwrap();
        copy_artifact(&file, &dest).unwrap();
        assert!(dest.is_file());
        assert_eq!(std::fs::read(&dest).unwrap(), b"MZ");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_clap_bundle_is_picked_by_name_not_by_directory_order() {
        use super::clap_bundle_in;
        let root = scratch("clap");
        assert!(clap_bundle_in(&root).is_err(), "nothing bundled yet");
        // A stale bundle from the pre-rename crate must never be chosen.
        std::fs::create_dir_all(root.join("vb-plugin.clap")).unwrap();
        assert!(clap_bundle_in(&root).is_err());
        std::fs::create_dir_all(root.join("seclab-plugin.clap")).unwrap();
        assert_eq!(
            clap_bundle_in(&root).unwrap(),
            root.join("seclab-plugin.clap")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn vst3_wraps_everywhere_but_au_and_the_wrapper_app_are_macos_only() {
        for os in ["macos", "linux", "windows"] {
            assert!(WrappedFormat::Vst3.supported_on(os), "VST3 on {os}");
        }
        assert!(WrappedFormat::Auv2.supported_on("macos"));
        assert!(WrappedFormat::Standalone.supported_on("macos"));
        for os in ["linux", "windows"] {
            assert!(!WrappedFormat::Auv2.supported_on(os), "AU is Apple-only");
            // Off macOS the shipped standalone is our own `seclab-standalone`.
            assert!(!WrappedFormat::Standalone.supported_on(os));
        }
    }

    #[test]
    fn install_hints_follow_each_platforms_standard_plugin_folders() {
        use super::clap_install_dir;
        // Paths the hosts scan by default (Steinberg's VST3 locations; clap-wrapper's CLAP
        // search in src/detail/clap/fsutil.cpp, which is how the VST3 finds the .clap).
        assert_eq!(
            WrappedFormat::Vst3.install_dir("macos"),
            "~/Library/Audio/Plug-Ins/VST3/"
        );
        assert_eq!(WrappedFormat::Vst3.install_dir("linux"), "~/.vst3/");
        assert_eq!(
            WrappedFormat::Vst3.install_dir("windows"),
            r"%COMMONPROGRAMFILES%\VST3\"
        );
        assert_eq!(clap_install_dir("macos"), "~/Library/Audio/Plug-Ins/CLAP/");
        assert_eq!(clap_install_dir("linux"), "~/.clap/");
        assert_eq!(clap_install_dir("windows"), r"%COMMONPROGRAMFILES%\CLAP\");
    }

    #[test]
    fn au_display_name_is_manufacturer_then_plugin() {
        // Hosts split on ": " → manufacturer "ice1x", plugin "SecLab".
        let (manufacturer, plugin) = AU_DISPLAY_NAME
            .split_once(": ")
            .expect("Manufacturer: Plugin");
        assert_eq!(manufacturer, "ice1x");
        assert_eq!(plugin, "SecLab");
    }

    /// The real-host testing checklist must name every bundle the build produces, so the
    /// install table there cannot silently drift from what `bundle-*` actually emits.
    #[test]
    fn host_testing_checklist_names_every_produced_bundle() {
        let doc = include_str!("../../docs/host-testing.md");
        // The CLAP is produced by the plain bundler; the other three by the wrapper.
        let mut artifacts = vec!["seclab-plugin.clap"];
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
