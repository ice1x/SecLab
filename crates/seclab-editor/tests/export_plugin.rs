//! End-to-end "Export plugin" journey (000267): a user opens a factory patch in the editor,
//! tweaks it, names it in the Export dialog and exports — and the installed bundles carry
//! exactly the patch on the canvas, under the plugin identity the dialog previewed.
//!
//! Runs the real `seclab_export::export_plugin` against stock-looking template bundles in a temp
//! folder (no signing, no real plug-in folders), driven through the same `EditorApp` API the
//! toolbar button and dialog use.
#![cfg(feature = "export")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use seclab_editor::export_panel::{ExportHost, Exporter};
use seclab_editor::EditorApp;
use seclab_export::bundle::{NoSign, PATCH_FILE, TEMPLATE_STEM};
use seclab_export::identity::IDENTITY_FILE;
use seclab_export::{export_plugin, PluginDirs, PluginIdentity};

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn plist(path: &Path, pairs: &[(&str, plist::Value)]) {
    let dict: plist::Dictionary = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    plist::Value::Dictionary(dict).to_file_xml(path).unwrap();
}

fn bundle(path: &Path, extra: &[(&str, plist::Value)]) {
    let macos = path.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    std::fs::write(macos.join(TEMPLATE_STEM), b"bin").unwrap();
    let mut pairs = vec![(
        "CFBundleExecutable",
        plist::Value::String(TEMPLATE_STEM.into()),
    )];
    pairs.extend_from_slice(extra);
    plist(&path.join("Contents/Info.plist"), &pairs);
}

/// A temp "home" with the stock SecLab CLAP + VST3 + AU installed.
fn installed_home(tag: &str) -> (Root, PluginDirs) {
    let root = std::env::temp_dir().join(format!("seclab-e2e-export-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dirs = PluginDirs::user_macos(&root);
    let t = dirs.templates();
    bundle(&t.clap, &[]);
    bundle(&t.vst3, &[]);
    let au: plist::Dictionary = [
        ("name", "ice1x: SecLab"),
        ("subtype", "gWrp"),
        ("manufacturer", "clAd"),
        ("type", "aumu"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), plist::Value::String(v.into())))
    .collect();
    bundle(
        &t.component,
        &[(
            "AudioComponents",
            plist::Value::Array(vec![plist::Value::Dictionary(au)]),
        )],
    );
    (Root(root), dirs)
}

fn exporter_into(dirs: &PluginDirs) -> Exporter {
    let dirs = dirs.clone();
    Arc::new(move |identity, ron| export_plugin(identity, ron, &dirs.templates(), &dirs, &NoSign))
}

fn finish(app: &mut EditorApp) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.export_dialog().in_flight() && Instant::now() < deadline {
        app.export_dialog_mut().poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.export_dialog().in_flight(), "export did not finish");
}

fn factory_patch(name: &str) -> seclab_project::ProjectDoc {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    seclab_project::open_path(&path).unwrap()
}

#[test]
fn exporting_a_tweaked_factory_patch_installs_it_as_its_own_plugin() {
    let (_root, dirs) = installed_home("tweak");
    let mut app = EditorApp::new();
    app.load_doc(&factory_patch("minitaur.ron"));
    // The user tweaks a knob before exporting: the export must carry the edited value.
    let doc = app.patch().to_doc();
    let entry = doc.nodes.iter().find(|e| !e.values.is_empty()).unwrap();
    let mut values = entry.values.clone();
    values[0] += 1.0;
    let mut edited = doc.clone();
    assert!(edited.set_node_values(entry.id, values));
    app.load_doc(&edited);

    *app.export_dialog_mut() =
        ExportHost::with_exporter(exporter_into(&dirs), Some(dirs.clap.clone()), true);
    app.export_dialog_mut().open();
    app.export_dialog_mut().set_name("My Minitaur");
    let previewed = app.export_dialog().preview().unwrap();
    assert!(!app.export_dialog().replaces_existing());
    app.start_export();
    finish(&mut app);

    let out = app
        .export_dialog()
        .exported()
        .expect("export succeeded")
        .clone();
    assert_eq!(out.identity, previewed);
    // The CLAP carries the patch exactly as it is on the canvas, and the identity.
    let resources = out.clap.join("Contents/Resources");
    let shipped = std::fs::read_to_string(resources.join(PATCH_FILE)).unwrap();
    assert_eq!(
        seclab_project::open_ron(&shipped).unwrap(),
        app.patch().to_doc()
    );
    let id = std::fs::read_to_string(resources.join(IDENTITY_FILE)).unwrap();
    assert_eq!(PluginIdentity::from_ron(&id).unwrap(), previewed);
    // …and so does the copy embedded in each wrapper.
    for wrapper in [out.vst3.unwrap(), out.component.unwrap()] {
        let inner = wrapper.join(format!("Contents/PlugIns/{TEMPLATE_STEM}.clap"));
        let embedded = std::fs::read_to_string(inner.join("Contents/Resources").join(PATCH_FILE));
        assert_eq!(embedded.unwrap(), shipped, "{}", wrapper.display());
    }
}

#[test]
fn two_patches_become_two_plugins_and_re_export_updates_one_of_them() {
    let (_root, dirs) = installed_home("two");
    let mut app = EditorApp::new();
    *app.export_dialog_mut() =
        ExportHost::with_exporter(exporter_into(&dirs), Some(dirs.clap.clone()), true);

    for (name, patch) in [("Bass", "minitaur.ron"), ("Lead", "first_synth.ron")] {
        app.load_doc(&factory_patch(patch));
        app.export_dialog_mut().open();
        app.export_dialog_mut().set_name(name);
        app.start_export();
        finish(&mut app);
        assert!(app.export_dialog().exported().is_some(), "{name}");
    }
    let ids: Vec<String> = ["Bass", "Lead"]
        .iter()
        .map(|n| {
            let f = dirs
                .clap
                .join(format!("{n}.clap/Contents/Resources/{IDENTITY_FILE}"));
            PluginIdentity::from_ron(&std::fs::read_to_string(f).unwrap())
                .unwrap()
                .clap_id
        })
        .collect();
    assert_ne!(ids[0], ids[1], "each export is a distinct plugin");

    // Re-exporting "Bass" from a different patch is flagged, then replaces only "Bass".
    app.load_doc(&factory_patch("first_synth.ron"));
    app.export_dialog_mut().open();
    app.export_dialog_mut().set_name("Bass");
    assert!(app.export_dialog().replaces_existing());
    app.start_export();
    finish(&mut app);
    let bass = std::fs::read_to_string(
        dirs.clap
            .join("Bass.clap/Contents/Resources")
            .join(PATCH_FILE),
    );
    assert_eq!(
        seclab_project::open_ron(&bass.unwrap()).unwrap(),
        app.patch().to_doc()
    );
    assert!(dirs.clap.join("Lead.clap").is_dir());
    // The stock plugin is still there, untouched.
    assert!(!dirs.templates().clap.join("Contents/Resources").exists());
}

#[test]
fn without_the_stock_plugin_installed_the_dialog_explains_and_nothing_is_written() {
    let root = std::env::temp_dir().join(format!("seclab-e2e-export-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _cleanup = Root(root.clone());
    let dirs = PluginDirs::user_macos(&root);
    let mut app = EditorApp::new();
    *app.export_dialog_mut() =
        ExportHost::with_exporter(exporter_into(&dirs), Some(dirs.clap.clone()), true);
    app.export_dialog_mut().set_name("Bass");
    app.start_export();
    finish(&mut app);
    assert!(app
        .export_dialog()
        .error()
        .unwrap()
        .contains("not installed"));
    assert!(!dirs.clap.exists());
}
