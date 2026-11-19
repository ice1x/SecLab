//! Public-beta release packaging for macOS: ad-hoc signing, per-format zips, a `.pkg`
//! installer, and the GitHub Release notes.
//!
//! Like `sign.rs`, the order-sensitive bits — tool argument vectors, the installer's
//! distribution XML, asset names, the CHANGELOG section a release quotes — are pure,
//! unit-tested functions; `cargo xtask package-macos` / `release-notes` only shell out.

use std::path::Path;

/// One installable part of the macOS release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    /// Short id: the installer choice id and the `.pkg` identifier suffix.
    pub id: &'static str,
    /// The bundle's name in `target/bundled/`.
    pub bundle: &'static str,
    /// Where the installer puts it (system-wide).
    pub install_location: &'static str,
    /// Label in asset names (`SecLab-<v>-macOS-arm64-<label>.zip`).
    pub label: &'static str,
    /// Installer choice title.
    pub title: &'static str,
    /// Installer choice description.
    pub description: &'static str,
    /// Whether the user may deselect it in the installer.
    pub optional: bool,
}

/// What the macOS release ships. The CLAP comes first and is mandatory: the stock VST3, AU and
/// standalone wrappers load it from the CLAP folder at runtime.
pub const MACOS_COMPONENTS: [Component; 4] = [
    Component {
        id: "clap",
        bundle: "seclab-plugin.clap",
        install_location: "/Library/Audio/Plug-Ins/CLAP",
        label: "CLAP",
        title: "SecLab CLAP plug-in (required)",
        description: "The SecLab engine and editor. The VST3, AU and standalone app load it, so it is always installed.",
        optional: false,
    },
    Component {
        id: "vst3",
        bundle: "seclab-plugin.vst3",
        install_location: "/Library/Audio/Plug-Ins/VST3",
        label: "VST3",
        title: "SecLab VST3 plug-in",
        description: "For Ableton Live, Cubase, Studio One, FL Studio and other VST3 hosts.",
        optional: true,
    },
    Component {
        id: "au",
        bundle: "seclab-plugin.component",
        install_location: "/Library/Audio/Plug-Ins/Components",
        label: "AU",
        title: "SecLab Audio Unit (AUv2)",
        description: "For Logic Pro, GarageBand, MainStage and other Audio Unit hosts.",
        optional: true,
    },
    Component {
        id: "app",
        bundle: "seclab-plugin.app",
        install_location: "/Applications",
        label: "Standalone",
        title: "SecLab standalone app",
        description: "Play and edit patches without a DAW.",
        optional: true,
    },
];

/// The license documents shipped next to the bundles in every archive.
pub const LICENSE_FILES: [&str; 3] = ["LICENSE", "NOTICE", "EULA-BETA.md"];

/// Installer package identifiers live under this prefix.
pub const PKG_ID_PREFIX: &str = "com.ice1x.seclab.pkg";

/// Refreshes the Audio Unit cache after an install so hosts see the new component without a
/// reboot (the registrar is relaunched on demand).
pub const AU_POSTINSTALL: &str =
    "#!/bin/sh\nkillall -9 AudioComponentRegistrar >/dev/null 2>&1 || true\nexit 0\n";

/// The version a tag names: `v0.1.0-beta.1` → `0.1.0-beta.1`; a branch (manual run) has its
/// `/` replaced so it can be part of a file name.
#[must_use]
pub fn release_version(tag: &str) -> String {
    tag.strip_prefix('v')
        .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(tag)
        .replace('/', "-")
}

/// `SecLab-<version>-macOS-arm64[-<label>].<ext>`.
#[must_use]
pub fn macos_asset_name(version: &str, label: Option<&str>, ext: &str) -> String {
    match label {
        Some(l) => format!("SecLab-{version}-macOS-arm64-{l}.{ext}"),
        None => format!("SecLab-{version}-macOS-arm64.{ext}"),
    }
}

/// The installer package identifier of a component.
#[must_use]
pub fn pkg_identifier(c: &Component) -> String {
    format!("{PKG_ID_PREFIX}.{}", c.id)
}

/// `codesign` arguments for an ad-hoc signature (no Developer ID): seals the whole bundle so
/// Apple-silicon hosts accept it once the user has cleared Gatekeeper. Not notarizable.
#[must_use]
pub fn adhoc_codesign_args(bundle: &Path) -> Vec<String> {
    vec![
        "--force".to_owned(),
        "--deep".to_owned(),
        "--sign".to_owned(),
        "-".to_owned(),
        "--timestamp=none".to_owned(),
        bundle.display().to_string(),
    ]
}

/// The component property list for `pkgbuild --component-plist`. Bundles are marked
/// **non-relocatable**: by default Installer "upgrades" a bundle in place wherever it finds one
/// with the same identifier — e.g. a developer build in `~/Library` — instead of installing to
/// the stated location.
#[must_use]
pub fn component_plist(bundle: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
	<dict>
		<key>BundleHasStrictIdentifier</key>
		<false/>
		<key>BundleIsRelocatable</key>
		<false/>
		<key>BundleIsVersionChecked</key>
		<false/>
		<key>BundleOverwriteAction</key>
		<string>upgrade</string>
		<key>RootRelativeBundlePath</key>
		<string>{}</string>
	</dict>
</array>
</plist>
"#,
        xml_escape(bundle)
    )
}

/// `pkgbuild` arguments for one component package.
#[must_use]
pub fn pkgbuild_args(
    c: &Component,
    root: &Path,
    component_plist: &Path,
    scripts: Option<&Path>,
    version: &str,
    out: &Path,
) -> Vec<String> {
    let mut args = vec![
        "--root".to_owned(),
        root.display().to_string(),
        "--component-plist".to_owned(),
        component_plist.display().to_string(),
        "--identifier".to_owned(),
        pkg_identifier(c),
        "--version".to_owned(),
        version.to_owned(),
        "--install-location".to_owned(),
        c.install_location.to_owned(),
    ];
    if let Some(s) = scripts {
        args.push("--scripts".to_owned());
        args.push(s.display().to_string());
    }
    args.push(out.display().to_string());
    args
}

/// The component package file name inside the product archive.
#[must_use]
pub fn component_pkg_name(c: &Component) -> String {
    format!("SecLab-{}.pkg", c.id)
}

/// The installer's distribution XML: a title, the beta EULA as the license page, Apple-silicon
/// only, and one choice per component (the CLAP locked on).
#[must_use]
pub fn distribution_xml(version: &str, components: &[Component], license_file: &str) -> String {
    let v = xml_escape(version);
    let mut outline = String::new();
    let mut choices = String::new();
    for c in components {
        let id = pkg_identifier(c);
        outline.push_str(&format!("        <line choice=\"{}\"/>\n", c.id));
        let lock = if c.optional {
            ""
        } else {
            " enabled=\"false\" selected=\"true\""
        };
        choices.push_str(&format!(
            "    <choice id=\"{}\" title=\"{}\" description=\"{}\"{lock}>\n        <pkg-ref id=\"{id}\"/>\n    </choice>\n    <pkg-ref id=\"{id}\" version=\"{v}\" onConclusion=\"none\">{}</pkg-ref>\n",
            c.id,
            xml_escape(c.title),
            xml_escape(c.description),
            component_pkg_name(c),
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>SecLab {v} (public beta)</title>
    <license file="{}"/>
    <options customize="allow" require-scripts="false" hostArchitectures="arm64"/>
    <domains enable_localSystem="true"/>
    <volume-check>
        <allowed-os-versions>
            <os-version min="11.0"/>
        </allowed-os-versions>
    </volume-check>
    <choices-outline>
{outline}    </choices-outline>
{choices}</installer-gui-script>
"#,
        xml_escape(license_file)
    )
}

/// `productbuild` arguments that wrap the component packages into the final installer.
#[must_use]
pub fn productbuild_args(
    distribution: &Path,
    package_path: &Path,
    resources: &Path,
    version: &str,
    out: &Path,
) -> Vec<String> {
    vec![
        "--distribution".to_owned(),
        distribution.display().to_string(),
        "--package-path".to_owned(),
        package_path.display().to_string(),
        "--resources".to_owned(),
        resources.display().to_string(),
        "--identifier".to_owned(),
        PKG_ID_PREFIX.to_owned(),
        "--version".to_owned(),
        version.to_owned(),
        out.display().to_string(),
    ]
}

/// The workspace version (`[workspace.package] version`) in the root `Cargo.toml` text.
#[must_use]
pub fn workspace_version(cargo_toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in cargo_toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
        } else if in_package {
            if let Some(rest) = line.strip_prefix("version") {
                let value = rest.trim_start().strip_prefix('=')?.trim();
                return Some(value.trim_matches('"').to_owned());
            }
        }
    }
    None
}

/// A tag must name the version being built, or the release would carry a mislabelled build.
///
/// # Errors
/// When the tag is a version tag (`v…`) and differs from the workspace version.
pub fn check_tag_matches(tag: &str, workspace: &str) -> Result<(), String> {
    let is_version_tag = tag
        .strip_prefix('v')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    let version = release_version(tag);
    if is_version_tag && version != workspace {
        return Err(format!(
            "tag {tag} does not match the workspace version {workspace}; bump Cargo.toml first"
        ));
    }
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The body of `## [<version>]` in a Keep-a-Changelog file (without the heading), trimmed.
#[must_use]
pub fn changelog_section<'a>(changelog: &'a str, version: &str) -> Option<&'a str> {
    let heading = format!("## [{version}]");
    let mut start = None;
    let mut offset = 0;
    for line in changelog.split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        if let Some(s) = start {
            if line.starts_with("## ") {
                return Some(changelog[s..at].trim());
            }
        } else if line.trim_end() == heading || line.starts_with(&format!("{heading} ")) {
            start = Some(offset);
        }
    }
    start.map(|s| changelog[s..].trim())
}

/// The `### Highlights` part of a section, when it has one (long sections quote only that).
#[must_use]
pub fn highlights(section: &str) -> Option<&str> {
    let start = section.find("### Highlights")?;
    let body = &section[start + "### Highlights".len()..];
    let end = body.find("\n### ").unwrap_or(body.len());
    Some(body[..end].trim())
}

/// The GitHub Release body for `version`: its CHANGELOG notes (the highlights when the section
/// has them, with a link to the rest), then how to install the beta.
///
/// # Errors
/// When the CHANGELOG has no `## [<version>]` section — cut the release in the CHANGELOG first.
pub fn release_notes(changelog: &str, version: &str, repo: &str) -> Result<String, String> {
    let section = changelog_section(changelog, version).ok_or_else(|| {
        format!("CHANGELOG.md has no `## [{version}]` section; move the Unreleased notes under it first")
    })?;
    let notes = match highlights(section) {
        Some(h) => format!(
            "## Highlights\n\n{h}\n\nFull list of changes: [CHANGELOG.md](https://github.com/{repo}/blob/v{version}/CHANGELOG.md)."
        ),
        None => section.to_owned(),
    };
    let pkg = macos_asset_name(version, None, "pkg");
    Ok(format!(
        "{notes}\n\n{}",
        INSTALL_NOTES
            .replace("{pkg}", &pkg)
            .replace("{repo}", repo)
            .trim_end()
    ))
}

const INSTALL_NOTES: &str = r#"## Install

> **Public beta.** Free to use, including for music you release — see the [beta EULA](https://github.com/{repo}/blob/main/EULA-BETA.md). Please report bugs with **Report** in the app's toolbar or [GitHub Issues](https://github.com/{repo}/issues).

### macOS (Apple Silicon, macOS 11+)

**Installer (recommended):** download **`{pkg}`** and open it. It installs the CLAP, VST3, AU and the standalone app into `/Library/Audio/Plug-Ins` and `/Applications` (you can untick formats under *Customize*).

The beta is ad-hoc signed, not notarized by Apple, so the first time macOS will say it *cannot verify the developer*. Click **Done**, then open **System Settings → Privacy & Security**, scroll down and click **Open Anyway** next to the SecLab installer. Files installed by the `.pkg` need no further steps. Rescan plug-ins in your DAW.

**Manual install:** download the per-format `.zip`, move the bundle into the folder below, then remove the download quarantine (only for files from this release page):

```sh
sudo xattr -dr com.apple.quarantine "/Library/Audio/Plug-Ins/CLAP/seclab-plugin.clap"
sudo xattr -dr com.apple.quarantine "/Library/Audio/Plug-Ins/VST3/seclab-plugin.vst3"
sudo xattr -dr com.apple.quarantine "/Library/Audio/Plug-Ins/Components/seclab-plugin.component"
sudo xattr -dr com.apple.quarantine "/Applications/seclab-plugin.app"
```

The VST3, AU and standalone app load the CLAP at runtime — always install the CLAP too.

### Windows and Linux (x86-64)

The `Windows-x86_64` / `Linux-x86_64` zips contain the CLAP, a VST3 and the native standalone editor. Copy the `.clap` and `.vst3` into your CLAP and VST3 folders (Windows: `C:\Program Files\Common Files\CLAP` and `…\VST3`; Linux: `~/.clap` and `~/.vst3`). Builds are unsigned; Windows SmartScreen may warn on first launch (*More info → Run anyway*).

`SHA256SUMS.txt` lists the checksum of every file in this release.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_tag_names_its_version_and_a_branch_becomes_a_file_safe_name() {
        assert_eq!(release_version("v0.1.0-beta.1"), "0.1.0-beta.1");
        assert_eq!(release_version("v1.2.3"), "1.2.3");
        assert_eq!(release_version("feature/x"), "feature-x");
        assert_eq!(release_version("main"), "main");
        // A "v" that is not followed by a number is part of the name, not a version prefix.
        assert_eq!(release_version("very/new"), "very-new");
    }

    #[test]
    fn asset_names_say_product_version_platform_and_format() {
        assert_eq!(
            macos_asset_name("0.1.0-beta.1", Some("VST3"), "zip"),
            "SecLab-0.1.0-beta.1-macOS-arm64-VST3.zip"
        );
        assert_eq!(
            macos_asset_name("0.1.0-beta.1", None, "pkg"),
            "SecLab-0.1.0-beta.1-macOS-arm64.pkg"
        );
    }

    #[test]
    fn the_clap_is_first_and_the_only_mandatory_component() {
        assert_eq!(MACOS_COMPONENTS[0].bundle, "seclab-plugin.clap");
        assert!(!MACOS_COMPONENTS[0].optional);
        assert!(MACOS_COMPONENTS[1..].iter().all(|c| c.optional));
        let bundles: Vec<_> = MACOS_COMPONENTS.iter().map(|c| c.bundle).collect();
        assert_eq!(
            bundles,
            [
                "seclab-plugin.clap",
                "seclab-plugin.vst3",
                "seclab-plugin.component",
                "seclab-plugin.app"
            ]
        );
    }

    #[test]
    fn plug_ins_install_system_wide_and_the_app_into_applications() {
        for c in &MACOS_COMPONENTS {
            let expected = match c.id {
                "clap" => "/Library/Audio/Plug-Ins/CLAP",
                "vst3" => "/Library/Audio/Plug-Ins/VST3",
                "au" => "/Library/Audio/Plug-Ins/Components",
                "app" => "/Applications",
                other => panic!("unexpected component {other}"),
            };
            assert_eq!(c.install_location, expected);
        }
    }

    #[test]
    fn ad_hoc_signing_uses_the_dash_identity_without_a_timestamp() {
        assert_eq!(
            adhoc_codesign_args(Path::new("/b/seclab-plugin.vst3")),
            [
                "--force",
                "--deep",
                "--sign",
                "-",
                "--timestamp=none",
                "/b/seclab-plugin.vst3"
            ]
        );
    }

    #[test]
    fn bundles_are_never_relocated_to_another_copy_on_the_machine() {
        let plist = component_plist("seclab-plugin.vst3");
        let parsed = plist::Value::from_reader_xml(plist.as_bytes()).unwrap();
        let entry = parsed.as_array().unwrap()[0].as_dictionary().unwrap();
        assert_eq!(entry["BundleIsRelocatable"].as_boolean(), Some(false));
        assert_eq!(
            entry["RootRelativeBundlePath"].as_string(),
            Some("seclab-plugin.vst3")
        );
    }

    #[test]
    fn pkgbuild_gets_root_plist_identifier_version_location_and_optional_scripts() {
        let au = MACOS_COMPONENTS[2];
        let args = pkgbuild_args(
            &au,
            Path::new("/s/root"),
            Path::new("/s/c.plist"),
            Some(Path::new("/s/scripts")),
            "0.1.0",
            Path::new("/s/SecLab-au.pkg"),
        );
        assert_eq!(
            args,
            [
                "--root",
                "/s/root",
                "--component-plist",
                "/s/c.plist",
                "--identifier",
                "com.ice1x.seclab.pkg.au",
                "--version",
                "0.1.0",
                "--install-location",
                "/Library/Audio/Plug-Ins/Components",
                "--scripts",
                "/s/scripts",
                "/s/SecLab-au.pkg"
            ]
        );
        let vst3 = pkgbuild_args(
            &MACOS_COMPONENTS[1],
            Path::new("/r"),
            Path::new("/p"),
            None,
            "1",
            Path::new("/o.pkg"),
        );
        assert!(!vst3.contains(&"--scripts".to_owned()));
        assert_eq!(vst3.last().unwrap(), "/o.pkg");
    }

    #[test]
    fn the_au_postinstall_refreshes_the_component_cache_and_never_fails() {
        assert!(AU_POSTINSTALL.starts_with("#!/bin/sh\n"));
        assert!(AU_POSTINSTALL.contains("killall -9 AudioComponentRegistrar"));
        assert!(AU_POSTINSTALL.contains("|| true"));
        assert!(AU_POSTINSTALL.trim_end().ends_with("exit 0"));
    }

    #[test]
    fn the_distribution_offers_one_choice_per_component_with_the_clap_locked_on() {
        let xml = distribution_xml("0.1.0-beta.1", &MACOS_COMPONENTS, "EULA.txt");
        // Well-formed: parse it as a plist-free XML document via a tiny check — every opened
        // choice is closed, and the outline lists the components in order.
        assert_eq!(xml.matches("<choice ").count(), 4);
        assert_eq!(xml.matches("</choice>").count(), 4);
        let order: Vec<usize> = ["clap", "vst3", "au", "app"]
            .iter()
            .map(|id| xml.find(&format!("<line choice=\"{id}\"/>")).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));
        assert!(xml.contains("<choice id=\"clap\" title=\"SecLab CLAP plug-in (required)\""));
        assert!(xml.contains("enabled=\"false\" selected=\"true\">"));
        assert_eq!(xml.matches("enabled=\"false\"").count(), 1);
        assert!(xml.contains(
            "<pkg-ref id=\"com.ice1x.seclab.pkg.au\" version=\"0.1.0-beta.1\" onConclusion=\"none\">SecLab-au.pkg</pkg-ref>"
        ));
        assert!(xml.contains("<license file=\"EULA.txt\"/>"));
        assert!(xml.contains("hostArchitectures=\"arm64\""));
        assert!(xml.contains("<title>SecLab 0.1.0-beta.1 (public beta)</title>"));
    }

    #[test]
    fn distribution_text_is_xml_escaped() {
        let mut c = MACOS_COMPONENTS[1];
        c.title = "VST3 & \"more\" <stuff>";
        let xml = distribution_xml("1", &[c], "EULA.txt");
        assert!(xml.contains("title=\"VST3 &amp; &quot;more&quot; &lt;stuff&gt;\""));
    }

    #[test]
    fn productbuild_wraps_the_component_packages() {
        assert_eq!(
            productbuild_args(
                Path::new("/s/d.xml"),
                Path::new("/s/pkgs"),
                Path::new("/s/res"),
                "0.1.0",
                Path::new("/d/SecLab.pkg")
            ),
            [
                "--distribution",
                "/s/d.xml",
                "--package-path",
                "/s/pkgs",
                "--resources",
                "/s/res",
                "--identifier",
                "com.ice1x.seclab.pkg",
                "--version",
                "0.1.0",
                "/d/SecLab.pkg"
            ]
        );
    }

    const CHANGELOG: &str = "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Added\n\n- next\n\n## [0.1.0-beta.1] - 2026-11-20\n\n### Highlights\n\n- Export plugin\n- AI assistant\n\n### Added\n\n- detail A\n\n## [0.0.1] - 2026-09-14\n\n### Added\n\n- first\n";

    #[test]
    fn a_changelog_section_runs_to_the_next_release_heading() {
        let s = changelog_section(CHANGELOG, "0.1.0-beta.1").unwrap();
        assert!(s.starts_with("### Highlights"));
        assert!(s.ends_with("- detail A"));
        assert_eq!(
            changelog_section(CHANGELOG, "0.0.1").unwrap(),
            "### Added\n\n- first"
        );
        assert_eq!(
            changelog_section(CHANGELOG, "Unreleased").unwrap(),
            "### Added\n\n- next"
        );
        assert!(changelog_section(CHANGELOG, "0.1.0").is_none());
        assert!(changelog_section(CHANGELOG, "0.1.0-beta").is_none());
    }

    #[test]
    fn highlights_stop_at_the_next_subsection() {
        let s = changelog_section(CHANGELOG, "0.1.0-beta.1").unwrap();
        assert_eq!(highlights(s).unwrap(), "- Export plugin\n- AI assistant");
        assert!(highlights("### Added\n\n- x").is_none());
    }

    #[test]
    fn release_notes_quote_the_highlights_and_explain_installation() {
        let notes = release_notes(CHANGELOG, "0.1.0-beta.1", "ice1x/SecLab").unwrap();
        assert!(notes.starts_with("## Highlights\n\n- Export plugin\n- AI assistant"));
        assert!(
            !notes.contains("detail A"),
            "long sections quote only the highlights"
        );
        assert!(notes.contains("https://github.com/ice1x/SecLab/blob/v0.1.0-beta.1/CHANGELOG.md"));
        assert!(notes.contains("`SecLab-0.1.0-beta.1-macOS-arm64.pkg`"));
        assert!(notes.contains("Open Anyway"));
        assert!(notes.contains("xattr -dr com.apple.quarantine"));
        assert!(notes.contains("https://github.com/ice1x/SecLab/blob/main/EULA-BETA.md"));
        assert!(!notes.contains("{repo}") && !notes.contains("{pkg}"));
        // Without highlights the whole section is quoted.
        let old = release_notes(CHANGELOG, "0.0.1", "o/r").unwrap();
        assert!(old.starts_with("### Added\n\n- first\n\n## Install"));
    }

    #[test]
    fn the_workspace_version_is_read_from_workspace_package() {
        let toml = "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0-beta.1\"\nedition = \"2021\"\n\n[workspace.dependencies]\nversion = \"9\"\n";
        assert_eq!(workspace_version(toml).as_deref(), Some("0.1.0-beta.1"));
        assert_eq!(workspace_version("[package]\nversion = \"1\"\n"), None);
    }

    #[test]
    fn a_version_tag_must_match_the_workspace_version() {
        assert!(check_tag_matches("v0.1.0-beta.1", "0.1.0-beta.1").is_ok());
        let err = check_tag_matches("v0.1.0", "0.0.82").unwrap_err();
        assert!(err.contains("0.0.82"));
        // A manual run from a branch is not a release; nothing to match.
        assert!(check_tag_matches("feature/x", "0.0.82").is_ok());
        assert!(check_tag_matches("very/new", "0.0.82").is_ok());
    }

    #[test]
    fn a_release_without_a_changelog_section_is_refused() {
        let err = release_notes(CHANGELOG, "9.9.9", "o/r").unwrap_err();
        assert!(err.contains("## [9.9.9]"));
    }
}
