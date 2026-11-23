//! The release version in the macOS bundles' `Info.plist`.
//!
//! Neither bundler knows the SecLab version on its own: nih-plug's bundler hardcodes
//! `1.0.0` into the CLAP, and clap-wrapper falls back to its own project version (`0.16.0`)
//! for the AU and the app — and, for the VST3, leaves `CFBundleShortVersionString` empty
//! and `CFBundleVersion` set to the literal `MACOSX_BUNDLE_SHORT_VERSION_STRING` (an empty
//! `BUNDLE_VERSION` shifts its `set_target_properties` list by one). `cargo xtask bundle*`
//! therefore passes the version to clap-wrapper (`CLAP_WRAPPER_BUNDLE_VERSION`) and then
//! stamps every bundle's `Info.plist` with [`stamp_bundle`], so the Finder, the installer
//! receipt and the AU registry all show the workspace version.

use std::path::Path;

/// The `MAJOR.MINOR.PATCH` core of a SemVer version — what Apple's
/// `CFBundleShortVersionString` / `CFBundleVersion` accept (period-separated integers, no
/// `-beta.1` pre-release or `+build` suffix).
pub fn bundle_version(version: &str) -> String {
    let end = version.find(['-', '+']).unwrap_or(version.len());
    version[..end].to_owned()
}

/// The AU `AudioComponents` `version` integer for `version`: `0xMMMMmmbb`, computed exactly
/// as clap-wrapper's AU build helper does (`(major << 16) + (minor << 8) + patch`, at least
/// 1), so the stamped plist agrees with the one clap-wrapper generates.
pub fn au_version(version: &str) -> u32 {
    let core = bundle_version(version);
    let mut parts = core.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    let (major, minor, patch) = (next(), next(), next());
    ((major << 16) + (minor << 8) + patch).max(1)
}

/// Writes `version` into an `Info.plist` dictionary: the numeric core into
/// `CFBundleShortVersionString` and `CFBundleVersion`, the full version (with its
/// pre-release label) into `CFBundleLongVersionString`, and — for an AU — the integer
/// version of every `AudioComponents` entry. Every other key is left as it is.
pub fn stamp(info: &mut plist::Dictionary, version: &str) {
    let core = bundle_version(version);
    for key in ["CFBundleShortVersionString", "CFBundleVersion"] {
        info.insert(key.to_owned(), plist::Value::String(core.clone()));
    }
    info.insert(
        "CFBundleLongVersionString".to_owned(),
        plist::Value::String(version.to_owned()),
    );
    if let Some(components) = info
        .get_mut("AudioComponents")
        .and_then(plist::Value::as_array_mut)
    {
        let integer = plist::Value::Integer(au_version(version).into());
        for component in components
            .iter_mut()
            .filter_map(plist::Value::as_dictionary_mut)
        {
            component.insert("version".to_owned(), integer.clone());
        }
    }
}

/// Stamps `version` into `<bundle>/Contents/Info.plist`. Returns `false` (and writes
/// nothing) when the bundle has no `Info.plist` — e.g. a Linux `.clap`, which is a single
/// shared library, not a bundle directory.
///
/// # Errors
/// When the `Info.plist` exists but cannot be read, parsed or written.
pub fn stamp_bundle(bundle: &Path, version: &str) -> Result<bool, String> {
    let path = bundle.join("Contents").join("Info.plist");
    if !path.is_file() {
        return Ok(false);
    }
    let fail = |what: &str, e: plist::Error| format!("{what} {}: {e}", path.display());
    let mut info = plist::Value::from_file(&path)
        .map_err(|e| fail("cannot read", e))?
        .into_dictionary()
        .ok_or_else(|| format!("{} is not a dictionary", path.display()))?;
    stamp(&mut info, version);
    plist::Value::Dictionary(info)
        .to_file_xml(&path)
        .map_err(|e| fail("cannot write", e))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::Value;

    #[test]
    fn the_bundle_version_is_the_numeric_core() {
        assert_eq!(bundle_version("0.1.0-beta.1"), "0.1.0");
        assert_eq!(bundle_version("1.2.3+build.7"), "1.2.3");
        assert_eq!(bundle_version("1.2.3-rc.1+build.7"), "1.2.3");
        assert_eq!(bundle_version("0.0.84"), "0.0.84");
    }

    #[test]
    fn the_au_version_matches_clap_wrappers_encoding() {
        assert_eq!(au_version("0.1.0-beta.1"), 0x0000_0100);
        assert_eq!(au_version("1.2.3"), 0x0001_0203);
        // clap-wrapper's 0.16.0 default — the value the stock AU carried before.
        assert_eq!(au_version("0.16.0"), 4096);
        // Never 0 (clap-wrapper clamps to 1).
        assert_eq!(au_version("0.0.0"), 1);
    }

    fn dict(pairs: &[(&str, Value)]) -> plist::Dictionary {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn string(info: &plist::Dictionary, key: &str) -> String {
        info.get(key).and_then(Value::as_string).unwrap().to_owned()
    }

    #[test]
    fn stamping_fixes_the_vst3_placeholder_and_keeps_other_keys() {
        // What clap-wrapper produced for the 0.1.0-beta.1 VST3.
        let mut info = dict(&[
            ("CFBundleExecutable", Value::String("seclab-plugin".into())),
            ("CFBundleLongVersionString", Value::String(String::new())),
            ("CFBundleShortVersionString", Value::String(String::new())),
            (
                "CFBundleVersion",
                Value::String("MACOSX_BUNDLE_SHORT_VERSION_STRING".into()),
            ),
        ]);
        stamp(&mut info, "0.1.0-beta.1");
        assert_eq!(string(&info, "CFBundleShortVersionString"), "0.1.0");
        assert_eq!(string(&info, "CFBundleVersion"), "0.1.0");
        assert_eq!(string(&info, "CFBundleLongVersionString"), "0.1.0-beta.1");
        assert_eq!(string(&info, "CFBundleExecutable"), "seclab-plugin");
        assert_eq!(info.len(), 4);
    }

    #[test]
    fn stamping_an_au_sets_every_component_version() {
        let component = |name: &str| {
            Value::Dictionary(dict(&[
                ("name", Value::String(name.into())),
                ("version", Value::Integer(4096.into())),
            ]))
        };
        let mut info = dict(&[
            ("CFBundleShortVersionString", Value::String("0.16.0".into())),
            (
                "AudioComponents",
                Value::Array(vec![component("ice1x: SecLab"), component("other")]),
            ),
        ]);
        stamp(&mut info, "1.2.3");
        let components = info.get("AudioComponents").unwrap().as_array().unwrap();
        for c in components {
            let c = c.as_dictionary().unwrap();
            assert_eq!(
                c.get("version").unwrap().as_unsigned_integer(),
                Some(0x10203)
            );
        }
        let first = components[0].as_dictionary().unwrap();
        assert_eq!(
            first.get("name").unwrap().as_string(),
            Some("ice1x: SecLab")
        );
        assert_eq!(string(&info, "CFBundleShortVersionString"), "1.2.3");
    }

    struct Dir(std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("xtask-version-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }

    #[test]
    fn a_bundle_on_disk_is_stamped_in_place() {
        let dir = temp("bundle");
        let bundle = dir.0.join("seclab-plugin.clap");
        let plist_path = bundle.join("Contents/Info.plist");
        std::fs::create_dir_all(plist_path.parent().unwrap()).unwrap();
        // nih-plug's bundler output.
        Value::Dictionary(dict(&[
            (
                "CFBundleIdentifier",
                Value::String("com.nih-plug.seclab-plugin".into()),
            ),
            ("CFBundleShortVersionString", Value::String("1.0.0".into())),
            ("CFBundleVersion", Value::String("1.0.0".into())),
        ]))
        .to_file_xml(&plist_path)
        .unwrap();

        assert_eq!(stamp_bundle(&bundle, "0.1.0-beta.1"), Ok(true));
        let info = Value::from_file(&plist_path)
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert_eq!(string(&info, "CFBundleShortVersionString"), "0.1.0");
        assert_eq!(string(&info, "CFBundleVersion"), "0.1.0");
        assert_eq!(
            string(&info, "CFBundleIdentifier"),
            "com.nih-plug.seclab-plugin"
        );
    }

    #[test]
    fn a_single_file_plugin_has_nothing_to_stamp() {
        let dir = temp("file");
        let so = dir.0.join("seclab-plugin.clap");
        std::fs::write(&so, b"ELF").unwrap();
        assert_eq!(stamp_bundle(&so, "0.1.0"), Ok(false));
        assert_eq!(std::fs::read(&so).unwrap(), b"ELF");
    }

    #[test]
    fn a_broken_info_plist_is_an_error() {
        let dir = temp("broken");
        let plist_path = dir.0.join("x.vst3/Contents/Info.plist");
        std::fs::create_dir_all(plist_path.parent().unwrap()).unwrap();
        std::fs::write(&plist_path, b"not a plist").unwrap();
        let err = stamp_bundle(&dir.0.join("x.vst3"), "0.1.0").unwrap_err();
        assert!(err.contains("Info.plist"), "{err}");
    }
}
