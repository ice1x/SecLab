//! Export a patch as its own plugin (000266): clone the installed SecLab bundles into
//! `<Name>.clap` / `<Name>.vst3` / `<Name>.component`, each carrying the patch and a
//! [`PluginIdentity`].
//!
//! No compiler is involved — the user's machine only needs the stock SecLab plugin installed
//! (the *templates*):
//!
//! - **CLAP** — a copy of `seclab-plugin.clap` with `Contents/Resources/{patch.vbp,
//!   identity.ron}` and its `Info.plist` bundle id/name rewritten. The plugin's own CLAP entry
//!   point reports the identity, so the host sees a separate plugin.
//! - **VST3 / AU** — clap-wrapper bundles load a CLAP by name at runtime, searching the
//!   wrapper's **own** `Contents/PlugIns` before the system CLAP folders
//!   (`getMacCLAPSearchPaths` → `CFBundleCopyBuiltInPlugInsURL`). So each exported wrapper
//!   embeds a copy of the exported CLAP as `Contents/PlugIns/<wrapper-executable>.clap` and plays
//!   *that* patch, not the stock one. The VST3 class id is derived by clap-wrapper from the CLAP
//!   id; the AU is told apart by a new `AudioComponents` subtype and name.
//!
//! Every changed bundle is re-signed through a [`Signer`] (ad-hoc `codesign` on macOS):
//! editing a signed bundle breaks its seal, and Apple-silicon hosts refuse unsigned code.

use std::path::{Path, PathBuf};

use crate::identity::{fnv1a32, IdentityError, PluginIdentity, BUILTIN_CLAP_ID, IDENTITY_FILE};

/// The patch file name inside a bundle's `Contents/Resources` (what the plugin loads).
pub const PATCH_FILE: &str = "patch.vbp";
/// File stem of the stock SecLab bundles the export clones.
pub const TEMPLATE_STEM: &str = "seclab-plugin";
/// The AU manufacturer prefix hosts show before the name (`ice1x: Acid Bass`).
pub const AU_NAME_PREFIX: &str = "ice1x: ";

/// The three plug-in folders a host scans (one per format).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDirs {
    /// CLAP bundles.
    pub clap: PathBuf,
    /// VST3 bundles.
    pub vst3: PathBuf,
    /// AudioUnit components.
    pub components: PathBuf,
}

impl PluginDirs {
    /// The per-user macOS plug-in folders under `home` (`~/Library/Audio/Plug-Ins/…`).
    #[must_use]
    pub fn user_macos(home: &Path) -> Self {
        let root = home.join("Library").join("Audio").join("Plug-Ins");
        Self {
            clap: root.join("CLAP"),
            vst3: root.join("VST3"),
            components: root.join("Components"),
        }
    }

    /// The stock SecLab bundles in these folders — the export's templates.
    #[must_use]
    pub fn templates(&self) -> Templates {
        Templates {
            clap: self.clap.join(format!("{TEMPLATE_STEM}.clap")),
            vst3: self.vst3.join(format!("{TEMPLATE_STEM}.vst3")),
            component: self.components.join(format!("{TEMPLATE_STEM}.component")),
        }
    }
}

/// The stock bundles cloned by an export. The CLAP is required; a missing VST3 or AU template
/// just skips that format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Templates {
    /// `seclab-plugin.clap` (built with the editor).
    pub clap: PathBuf,
    /// `seclab-plugin.vst3` (the clap-wrapper VST3).
    pub vst3: PathBuf,
    /// `seclab-plugin.component` (the clap-wrapper AUv2).
    pub component: PathBuf,
}

/// What an export installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// The identity the bundles present.
    pub identity: PluginIdentity,
    /// The installed `<Name>.clap`.
    pub clap: PathBuf,
    /// The installed `<Name>.vst3`, when a VST3 template was found.
    pub vst3: Option<PathBuf>,
    /// The installed `<Name>.component`, when an AU template was found.
    pub component: Option<PathBuf>,
    /// The AU subtype code given to the component (also set when no AU was produced).
    pub au_subtype: String,
}

/// Why an export failed. Nothing is installed unless every requested bundle was built.
#[derive(Debug)]
pub enum ExportError {
    /// The identity is invalid.
    Identity(IdentityError),
    /// The name or id would replace the stock SecLab plugin.
    Reserved,
    /// The stock SecLab CLAP is not installed (expected at this path).
    MissingTemplate(PathBuf),
    /// A file operation failed.
    Io(String),
    /// A template's `Info.plist` is missing or not the expected shape.
    Plist(String),
    /// Re-signing a bundle failed.
    Sign(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity(e) => write!(f, "{e}"),
            Self::Reserved => write!(f, "that name is reserved for the stock SecLab plugin"),
            Self::MissingTemplate(p) => write!(
                f,
                "SecLab plugin is not installed (expected {}); install it first",
                p.display()
            ),
            Self::Io(e) => write!(f, "file error: {e}"),
            Self::Plist(e) => write!(f, "bundle Info.plist: {e}"),
            Self::Sign(e) => write!(f, "code signing failed: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<IdentityError> for ExportError {
    fn from(e: IdentityError) -> Self {
        Self::Identity(e)
    }
}

fn io(context: &str, path: &Path, e: &std::io::Error) -> ExportError {
    ExportError::Io(format!("{context} {}: {e}", path.display()))
}

/// Re-signs a bundle after it was modified.
pub trait Signer {
    /// Signs `bundle` in place.
    ///
    /// # Errors
    /// [`ExportError::Sign`] when signing fails.
    fn sign(&self, bundle: &Path) -> Result<(), ExportError>;
}

/// Leaves bundles unsigned (tests; platforms without code signing).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoSign;

impl Signer for NoSign {
    fn sign(&self, _bundle: &Path) -> Result<(), ExportError> {
        Ok(())
    }
}

/// Ad-hoc signs with Apple's `codesign` (`--sign -`): no developer identity needed, and enough
/// for a host on the same Mac to load the bundle.
#[derive(Debug, Default, Clone, Copy)]
pub struct AdHocCodesign;

impl Signer for AdHocCodesign {
    fn sign(&self, bundle: &Path) -> Result<(), ExportError> {
        let out = std::process::Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(bundle)
            .output()
            .map_err(|e| ExportError::Sign(format!("cannot run codesign: {e}")))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(ExportError::Sign(format!(
                "{}: {}",
                bundle.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
}

/// The platform's default [`Signer`]: ad-hoc `codesign` on macOS, none elsewhere.
#[must_use]
pub fn default_signer() -> Box<dyn Signer> {
    if cfg!(target_os = "macos") {
        Box::new(AdHocCodesign)
    } else {
        Box::new(NoSign)
    }
}

/// The 4-character AU subtype for a CLAP id: lowercase letters/digits from its FNV-1a hash.
/// Stable per id (re-exports update the same AU) and never the stock `gWrp` (which has
/// capitals).
#[must_use]
pub fn au_subtype(clap_id: &str) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut h = fnv1a32(clap_id.as_bytes());
    (0..4)
        .map(|_| {
            let c = ALPHABET[(h % 36) as usize];
            h /= 36;
            char::from(c)
        })
        .collect()
}

/// The file stem of the exported bundles: the display name with characters a file name cannot
/// hold (`/`, `:`, controls) replaced, and no leading dot (which would hide the bundle).
#[must_use]
pub fn bundle_stem(name: &str) -> String {
    let stem: String = name
        .trim()
        .chars()
        .map(|c| {
            if c == '/' || c == ':' || c == '\\' || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    stem.trim_start_matches('.').trim().to_owned()
}

/// Exports `patch_ron` as the plugin `identity`, cloning `templates` and installing the result
/// into `dest`. Re-exporting the same name replaces the earlier export.
///
/// Every bundle is assembled in a staging folder first; only when all of them are built and
/// signed are they moved into `dest`, so a failure never leaves a half-written plugin behind.
///
/// # Errors
/// See [`ExportError`].
pub fn export_plugin(
    identity: &PluginIdentity,
    patch_ron: &str,
    templates: &Templates,
    dest: &PluginDirs,
    signer: &dyn Signer,
) -> Result<Exported, ExportError> {
    identity.validate()?;
    let stem = bundle_stem(&identity.name);
    if identity.clap_id == BUILTIN_CLAP_ID || stem.is_empty() || stem == TEMPLATE_STEM {
        return Err(ExportError::Reserved);
    }
    if !templates.clap.is_dir() {
        return Err(ExportError::MissingTemplate(templates.clap.clone()));
    }

    let stage = staging_dir()?;
    let result = build_and_install(identity, patch_ron, templates, dest, signer, &stage, &stem);
    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn staging_dir() -> Result<PathBuf, ExportError> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("seclab-export-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| io("cannot create", &dir, &e))?;
    Ok(dir)
}

fn build_and_install(
    identity: &PluginIdentity,
    patch_ron: &str,
    templates: &Templates,
    dest: &PluginDirs,
    signer: &dyn Signer,
    stage: &Path,
    stem: &str,
) -> Result<Exported, ExportError> {
    let subtype = au_subtype(&identity.clap_id);

    // 1. The CLAP: the template plus the patch and identity.
    let clap = stage.join(format!("{stem}.clap"));
    copy_tree(&templates.clap, &clap)?;
    let resources = clap.join("Contents").join("Resources");
    std::fs::create_dir_all(&resources).map_err(|e| io("cannot create", &resources, &e))?;
    write_file(&resources.join(PATCH_FILE), patch_ron)?;
    write_file(&resources.join(IDENTITY_FILE), &identity.to_ron()?)?;
    edit_plist(&clap, |dict| {
        set_str(dict, "CFBundleIdentifier", &identity.clap_id);
        set_str(dict, "CFBundleName", &identity.name);
        set_str(dict, "CFBundleDisplayName", &identity.name);
        Ok(())
    })?;
    signer.sign(&clap)?;

    // 2./3. The wrappers, each embedding that CLAP under the name it searches for.
    let vst3 = if templates.vst3.is_dir() {
        let out = stage.join(format!("{stem}.vst3"));
        build_wrapper(&templates.vst3, &out, &clap, signer, |dict| {
            set_str(
                dict,
                "CFBundleIdentifier",
                &format!("{}.vst3", identity.clap_id),
            );
            set_str(dict, "CFBundleName", &identity.name);
            Ok(())
        })?;
        Some(out)
    } else {
        None
    };
    let component = if templates.component.is_dir() {
        let out = stage.join(format!("{stem}.component"));
        build_wrapper(&templates.component, &out, &clap, signer, |dict| {
            set_str(
                dict,
                "CFBundleIdentifier",
                &format!("{}.auv2", identity.clap_id),
            );
            set_str(dict, "CFBundleName", &identity.name);
            let component = dict
                .get_mut("AudioComponents")
                .and_then(plist::Value::as_array_mut)
                .and_then(|list| list.first_mut())
                .and_then(plist::Value::as_dictionary_mut)
                .ok_or_else(|| ExportError::Plist("no AudioComponents entry".into()))?;
            set_str(
                component,
                "name",
                &format!("{AU_NAME_PREFIX}{}", identity.name),
            );
            set_str(component, "subtype", &subtype);
            set_str(
                component,
                "description",
                &format!("{} — SecLab patch", identity.name),
            );
            Ok(())
        })?;
        Some(out)
    } else {
        None
    };

    // 4. Everything built: install.
    let installed_clap = install(&clap, &dest.clap)?;
    let installed_vst3 = vst3.map(|b| install(&b, &dest.vst3)).transpose()?;
    let installed_component = component
        .map(|b| install(&b, &dest.components))
        .transpose()?;
    Ok(Exported {
        identity: identity.clone(),
        clap: installed_clap,
        vst3: installed_vst3,
        component: installed_component,
        au_subtype: subtype,
    })
}

/// Clones a clap-wrapper bundle and embeds `clap` where its loader looks first.
fn build_wrapper(
    template: &Path,
    out: &Path,
    clap: &Path,
    signer: &dyn Signer,
    edit: impl FnOnce(&mut plist::Dictionary) -> Result<(), ExportError>,
) -> Result<(), ExportError> {
    copy_tree(template, out)?;
    let executable = read_plist(out)?
        .as_dictionary()
        .and_then(|d| d.get("CFBundleExecutable"))
        .and_then(plist::Value::as_string)
        .map(str::to_owned)
        .ok_or_else(|| ExportError::Plist(format!("{}: no CFBundleExecutable", out.display())))?;
    let plugins = out.join("Contents").join("PlugIns");
    copy_tree(clap, &plugins.join(format!("{executable}.clap")))?;
    edit_plist(out, edit)?;
    signer.sign(out)
}

/// Moves a built bundle into `dir`, replacing an earlier export of the same name.
fn install(bundle: &Path, dir: &Path) -> Result<PathBuf, ExportError> {
    std::fs::create_dir_all(dir).map_err(|e| io("cannot create", dir, &e))?;
    let name = bundle
        .file_name()
        .ok_or_else(|| ExportError::Io(format!("bad bundle path {}", bundle.display())))?;
    let target = dir.join(name);
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| io("cannot replace", &target, &e))?;
    }
    // A rename is atomic but fails across volumes (temp dir vs. home); fall back to a copy.
    if std::fs::rename(bundle, &target).is_err() {
        copy_tree(bundle, &target)?;
    }
    Ok(target)
}

fn write_file(path: &Path, text: &str) -> Result<(), ExportError> {
    std::fs::write(path, text).map_err(|e| io("cannot write", path, &e))
}

fn plist_path(bundle: &Path) -> PathBuf {
    bundle.join("Contents").join("Info.plist")
}

fn read_plist(bundle: &Path) -> Result<plist::Value, ExportError> {
    let path = plist_path(bundle);
    plist::Value::from_file(&path)
        .map_err(|e| ExportError::Plist(format!("{}: {e}", path.display())))
}

fn edit_plist(
    bundle: &Path,
    edit: impl FnOnce(&mut plist::Dictionary) -> Result<(), ExportError>,
) -> Result<(), ExportError> {
    let mut value = read_plist(bundle)?;
    let path = plist_path(bundle);
    let dict = value
        .as_dictionary_mut()
        .ok_or_else(|| ExportError::Plist(format!("{}: not a dictionary", path.display())))?;
    edit(dict)?;
    value
        .to_file_xml(&path)
        .map_err(|e| ExportError::Plist(format!("{}: {e}", path.display())))
}

fn set_str(dict: &mut plist::Dictionary, key: &str, value: &str) {
    dict.insert(key.to_owned(), plist::Value::String(value.to_owned()));
}

/// Recursively copies a bundle directory (files keep their permissions, so executables stay
/// executable). Symlinks are recreated as links, not followed.
fn copy_tree(from: &Path, to: &Path) -> Result<(), ExportError> {
    if to.exists() {
        std::fs::remove_dir_all(to).map_err(|e| io("cannot replace", to, &e))?;
    }
    std::fs::create_dir_all(to).map_err(|e| io("cannot create", to, &e))?;
    let entries = std::fs::read_dir(from).map_err(|e| io("cannot read", from, &e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io("cannot read", from, &e))?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| io("cannot stat", &src, &e))?;
        if kind.is_dir() {
            copy_tree(&src, &dst)?;
        } else if kind.is_symlink() {
            let link = std::fs::read_link(&src).map_err(|e| io("cannot read link", &src, &e))?;
            symlink(&link, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| io("cannot copy", &src, &e))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(link: &Path, dst: &Path) -> Result<(), ExportError> {
    std::os::unix::fs::symlink(link, dst).map_err(|e| io("cannot link", dst, &e))
}

#[cfg(not(unix))]
fn symlink(link: &Path, dst: &Path) -> Result<(), ExportError> {
    std::fs::copy(link, dst)
        .map(|_| ())
        .map_err(|e| io("cannot copy", dst, &e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A temp root removed on drop.
    struct Root(PathBuf);
    impl Root {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("seclab-export-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct Recorder(RefCell<Vec<PathBuf>>);
    impl Signer for Recorder {
        fn sign(&self, bundle: &Path) -> Result<(), ExportError> {
            self.0.borrow_mut().push(bundle.to_owned());
            Ok(())
        }
    }

    fn fake_bundle(path: &Path, plist: plist::Dictionary) {
        let macos = path.join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(macos.join(TEMPLATE_STEM), b"binary").unwrap();
        plist::Value::Dictionary(plist)
            .to_file_xml(path.join("Contents").join("Info.plist"))
            .unwrap();
    }

    fn dict(pairs: &[(&str, plist::Value)]) -> plist::Dictionary {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn s(v: &str) -> plist::Value {
        plist::Value::String(v.to_owned())
    }

    /// Installs stock-looking templates into `dirs`, like a real SecLab install.
    fn install_templates(dirs: &PluginDirs, with_wrappers: bool) {
        let t = dirs.templates();
        fake_bundle(
            &t.clap,
            dict(&[
                ("CFBundleExecutable", s(TEMPLATE_STEM)),
                ("CFBundleIdentifier", s("com.nih-plug.seclab-plugin")),
                ("CFBundleName", s(TEMPLATE_STEM)),
            ]),
        );
        if with_wrappers {
            fake_bundle(
                &t.vst3,
                dict(&[
                    ("CFBundleExecutable", s(TEMPLATE_STEM)),
                    (
                        "CFBundleIdentifier",
                        s("org.cleveraudio.wrapper.seclab-plugin.vst3"),
                    ),
                ]),
            );
            let au = dict(&[
                ("name", s("ice1x: SecLab")),
                ("subtype", s("gWrp")),
                ("manufacturer", s("clAd")),
                ("type", s("aumu")),
                ("factoryFunction", s("wrapAsAUV2_inst0Factory")),
            ]);
            fake_bundle(
                &t.component,
                dict(&[
                    ("CFBundleExecutable", s(TEMPLATE_STEM)),
                    (
                        "CFBundleIdentifier",
                        s("org.cleveraudio.wrapper.seclab-plugin.auv2"),
                    ),
                    (
                        "AudioComponents",
                        plist::Value::Array(vec![plist::Value::Dictionary(au)]),
                    ),
                ]),
            );
        }
    }

    fn plist_of(bundle: &Path) -> plist::Dictionary {
        read_plist(bundle).unwrap().into_dictionary().unwrap()
    }

    fn get<'a>(d: &'a plist::Dictionary, key: &str) -> &'a str {
        d.get(key).and_then(plist::Value::as_string).unwrap()
    }

    const PATCH: &str = "(version: 1, nodes: [], edges: [])";

    fn acid() -> PluginIdentity {
        PluginIdentity::for_name("Acid Bass").unwrap()
    }

    #[test]
    fn an_export_installs_a_clap_that_carries_the_patch_and_identity() {
        let root = Root::new("clap");
        let dirs = PluginDirs::user_macos(&root.0);
        install_templates(&dirs, false);
        let out = export_plugin(&acid(), PATCH, &dirs.templates(), &dirs, &NoSign).unwrap();

        assert_eq!(out.clap, dirs.clap.join("Acid Bass.clap"));
        let res = out.clap.join("Contents").join("Resources");
        assert_eq!(
            std::fs::read_to_string(res.join(PATCH_FILE)).unwrap(),
            PATCH
        );
        let id = std::fs::read_to_string(res.join(IDENTITY_FILE)).unwrap();
        assert_eq!(PluginIdentity::from_ron(&id).unwrap(), acid());
        let p = plist_of(&out.clap);
        assert_eq!(
            get(&p, "CFBundleIdentifier"),
            "com.ice1x.seclab.user.acid-bass"
        );
        assert_eq!(get(&p, "CFBundleName"), "Acid Bass");
        // The binary is the template's, untouched.
        assert_eq!(
            std::fs::read(out.clap.join("Contents/MacOS").join(TEMPLATE_STEM)).unwrap(),
            b"binary"
        );
        // Without wrapper templates only the CLAP is produced; the stock CLAP is untouched.
        assert_eq!((out.vst3, out.component), (None, None));
        assert_eq!(
            get(&plist_of(&dirs.templates().clap), "CFBundleIdentifier"),
            "com.nih-plug.seclab-plugin"
        );
        assert!(!dirs.templates().clap.join("Contents/Resources").exists());
    }

    #[test]
    fn each_wrapper_embeds_the_exported_clap_where_clap_wrapper_looks_first() {
        let root = Root::new("wrappers");
        let dirs = PluginDirs::user_macos(&root.0);
        install_templates(&dirs, true);
        let out = export_plugin(&acid(), PATCH, &dirs.templates(), &dirs, &NoSign).unwrap();

        let vst3 = out.vst3.unwrap();
        let au = out.component.unwrap();
        assert_eq!(vst3, dirs.vst3.join("Acid Bass.vst3"));
        assert_eq!(au, dirs.components.join("Acid Bass.component"));
        for wrapper in [&vst3, &au] {
            // `<CFBundleExecutable>.clap` in Contents/PlugIns — the name clap-wrapper searches.
            let inner = wrapper.join("Contents/PlugIns/seclab-plugin.clap");
            let patch = inner.join("Contents/Resources").join(PATCH_FILE);
            assert_eq!(
                std::fs::read_to_string(patch).unwrap(),
                PATCH,
                "{wrapper:?}"
            );
            assert_eq!(
                get(&plist_of(&inner), "CFBundleIdentifier"),
                "com.ice1x.seclab.user.acid-bass"
            );
        }
        let v = plist_of(&vst3);
        assert_eq!(
            get(&v, "CFBundleIdentifier"),
            "com.ice1x.seclab.user.acid-bass.vst3"
        );
        assert_eq!(get(&v, "CFBundleName"), "Acid Bass");
    }

    #[test]
    fn the_au_gets_its_own_name_and_subtype_and_keeps_the_rest() {
        let root = Root::new("au");
        let dirs = PluginDirs::user_macos(&root.0);
        install_templates(&dirs, true);
        let out = export_plugin(&acid(), PATCH, &dirs.templates(), &dirs, &NoSign).unwrap();

        let p = plist_of(out.component.as_ref().unwrap());
        assert_eq!(
            get(&p, "CFBundleIdentifier"),
            "com.ice1x.seclab.user.acid-bass.auv2"
        );
        let c = p["AudioComponents"].as_array().unwrap()[0]
            .as_dictionary()
            .unwrap();
        assert_eq!(get(c, "name"), "ice1x: Acid Bass");
        assert_eq!(get(c, "subtype"), out.au_subtype);
        assert_ne!(out.au_subtype, "gWrp");
        // Type, manufacturer and the factory symbol must stay — the wrapper binary needs them.
        assert_eq!(get(c, "type"), "aumu");
        assert_eq!(get(c, "manufacturer"), "clAd");
        assert_eq!(get(c, "factoryFunction"), "wrapAsAUV2_inst0Factory");
    }

    #[test]
    fn every_changed_bundle_is_signed_inner_clap_before_its_wrapper() {
        let root = Root::new("sign");
        let dirs = PluginDirs::user_macos(&root.0);
        install_templates(&dirs, true);
        let rec = Recorder::default();
        export_plugin(&acid(), PATCH, &dirs.templates(), &dirs, &rec).unwrap();
        let signed: Vec<String> = rec
            .0
            .borrow()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // The CLAP is signed before it is embedded, so each wrapper seals a signed inner bundle.
        assert_eq!(
            signed,
            ["Acid Bass.clap", "Acid Bass.vst3", "Acid Bass.component"]
        );
    }

    #[test]
    fn re_exporting_a_name_replaces_the_earlier_export() {
        let root = Root::new("again");
        let dirs = PluginDirs::user_macos(&root.0);
        install_templates(&dirs, true);
        let t = dirs.templates();
        export_plugin(&acid(), PATCH, &t, &dirs, &NoSign).unwrap();
        let stale = dirs
            .clap
            .join("Acid Bass.clap/Contents/Resources/stale.txt");
        std::fs::write(&stale, "old").unwrap();

        let newer = "(version: 1, nodes: [], edges: [], comment: 2)";
        let out = export_plugin(&acid(), newer, &t, &dirs, &NoSign).unwrap();
        let patch = out.clap.join("Contents/Resources").join(PATCH_FILE);
        assert_eq!(std::fs::read_to_string(patch).unwrap(), newer);
        assert!(!stale.exists(), "the old bundle is replaced, not merged");
        let other = PluginIdentity::for_name("Pad").unwrap();
        export_plugin(&other, PATCH, &t, &dirs, &NoSign).unwrap();
        assert!(
            dirs.clap.join("Acid Bass.clap").is_dir(),
            "other exports stay"
        );
    }

    #[test]
    fn the_stock_plugin_cannot_be_overwritten() {
        let root = Root::new("reserved");
        let dirs = PluginDirs::user_macos(&root.0);
        install_templates(&dirs, true);
        let t = dirs.templates();
        for identity in [
            PluginIdentity::builtin(),
            PluginIdentity {
                name: TEMPLATE_STEM.into(),
                clap_id: "com.ice1x.seclab.user.x".into(),
            },
            PluginIdentity {
                name: "...".into(),
                clap_id: "com.ice1x.seclab.user.x".into(),
            },
        ] {
            assert!(
                matches!(
                    export_plugin(&identity, PATCH, &t, &dirs, &NoSign),
                    Err(ExportError::Reserved)
                ),
                "{identity:?}"
            );
        }
    }

    #[test]
    fn without_the_stock_clap_nothing_is_installed() {
        let root = Root::new("missing");
        let dirs = PluginDirs::user_macos(&root.0);
        let err = export_plugin(&acid(), PATCH, &dirs.templates(), &dirs, &NoSign).unwrap_err();
        assert!(matches!(err, ExportError::MissingTemplate(_)));
        assert!(err.to_string().contains("not installed"));
        assert!(!dirs.clap.exists());
    }

    #[test]
    fn a_failure_midway_installs_nothing() {
        let root = Root::new("atomic");
        let dirs = PluginDirs::user_macos(&root.0);
        install_templates(&dirs, true);
        // Break the AU template's plist so the last bundle fails to build.
        std::fs::write(plist_path(&dirs.templates().component), "not a plist").unwrap();
        let err = export_plugin(&acid(), PATCH, &dirs.templates(), &dirs, &NoSign).unwrap_err();
        assert!(matches!(err, ExportError::Plist(_)), "{err}");
        assert!(!dirs.clap.join("Acid Bass.clap").exists());
        assert!(!dirs.vst3.join("Acid Bass.vst3").exists());
    }

    #[test]
    fn a_signing_failure_is_reported_and_installs_nothing() {
        struct Fails;
        impl Signer for Fails {
            fn sign(&self, b: &Path) -> Result<(), ExportError> {
                Err(ExportError::Sign(b.display().to_string()))
            }
        }
        let root = Root::new("signfail");
        let dirs = PluginDirs::user_macos(&root.0);
        install_templates(&dirs, true);
        let err = export_plugin(&acid(), PATCH, &dirs.templates(), &dirs, &Fails).unwrap_err();
        assert!(matches!(err, ExportError::Sign(_)));
        assert!(!dirs.clap.join("Acid Bass.clap").exists());
    }

    #[test]
    fn au_subtypes_are_stable_lowercase_and_differ_per_id() {
        let a = au_subtype("com.ice1x.seclab.user.acid-bass");
        assert_eq!(a, au_subtype("com.ice1x.seclab.user.acid-bass"));
        assert_eq!(a.len(), 4);
        assert!(a
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
        assert_ne!(a, au_subtype("com.ice1x.seclab.user.pad"));
    }

    #[test]
    fn bundle_stems_are_safe_file_names() {
        assert_eq!(bundle_stem(" Acid Bass "), "Acid Bass");
        assert_eq!(bundle_stem("A/B:C"), "A-B-C");
        assert_eq!(bundle_stem(".hidden"), "hidden");
        assert_eq!(bundle_stem("Кислотный бас"), "Кислотный бас");
    }

    #[test]
    fn the_user_plugin_folders_are_under_library_audio() {
        let dirs = PluginDirs::user_macos(Path::new("/Users/u"));
        assert_eq!(dirs.clap, Path::new("/Users/u/Library/Audio/Plug-Ins/CLAP"));
        assert_eq!(dirs.vst3, Path::new("/Users/u/Library/Audio/Plug-Ins/VST3"));
        assert_eq!(
            dirs.templates().component,
            Path::new("/Users/u/Library/Audio/Plug-Ins/Components/seclab-plugin.component")
        );
    }
}
