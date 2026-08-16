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
//!
//! Everything else is forwarded to nih-plug's xtask untouched.

use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).peekable();
    match args.peek().map(String::as_str) {
        Some("export") => {
            args.next();
            let patch = args.next().ok_or("usage: cargo xtask export <patch.vbp>")?;
            export(Path::new(&patch))
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
