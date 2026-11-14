//! Export a patch file as its own plugin, cloning the SecLab bundles installed in
//! `~/Library/Audio/Plug-Ins` (the same path the editor's "Export plugin" button takes).
//!
//! ```text
//! cargo run -p seclab-export --example export_installed -- <patch.vbp> "<Name>" [dest-root]
//! ```
//!
//! With `dest-root`, the bundles land in `<dest-root>/Library/Audio/Plug-Ins/…` instead of the
//! real plug-in folders (for a dry run). The templates always come from the real install.

use std::path::PathBuf;

use seclab_export::{bundle::default_signer, export_plugin, PluginDirs, PluginIdentity};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: export_installed <patch.vbp> <Name> [dest-root]";
    let patch = PathBuf::from(args.next().ok_or(usage)?);
    let name = args.next().ok_or(usage)?;
    let home = PathBuf::from(std::env::var("HOME")?);
    let installed = PluginDirs::user_macos(&home);
    let dest = args.next().map_or_else(
        || installed.clone(),
        |root| PluginDirs::user_macos(&PathBuf::from(root)),
    );

    let identity = PluginIdentity::for_name(&name)?;
    let ron = std::fs::read_to_string(&patch)?;
    let out = export_plugin(
        &identity,
        &ron,
        &installed.templates(),
        &dest,
        &*default_signer(),
    )?;
    println!("exported {} ({})", out.identity.name, out.identity.clap_id);
    println!("  clap      {}", out.clap.display());
    if let Some(p) = &out.vst3 {
        println!("  vst3      {}", p.display());
    }
    if let Some(p) = &out.component {
        println!("  component {} (aumu {} clAd)", p.display(), out.au_subtype);
    }
    Ok(())
}
