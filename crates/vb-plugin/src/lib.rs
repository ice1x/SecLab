//! `vb-plugin` — the VST_bomb patch as a CLAP instrument (nih-plug).
//!
//! The scaffold (task 000064): a nih-plug plugin that loads a serialized project —
//! the path in the `VB_PATCH` environment variable, or the built-in default patch —
//! compiles it into the allocation-free [`vb_runtime::RtEngine`] at `initialize`,
//! and streams the patch's terminal audio node to the host in stereo. VST3/AUv2
//! arrive later via clap-wrapper (000069/000070); parameters, host MIDI, and the
//! embedded GUI are the next Epic 7 tasks.
//!
//! Build the CLAP with `cargo build -p vb-plugin --release`; the resulting
//! `libvb_plugin` cdylib renamed to `vb_bomb.clap` is a loadable CLAP bundle (the
//! one-command bundler is task 000072).

pub mod engine_builder;

use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;
use vb_core::NodeId;
use vb_nodes::{BuildContext, Registry};
use vb_project::ProjectDoc;
use vb_runtime::RtEngine;

pub use engine_builder::{default_patch, engine_from_doc, BuildError};

/// The VST_bomb instrument plugin.
pub struct VstBomb {
    params: Arc<VstBombParams>,
    /// The project this instance plays.
    doc: ProjectDoc,
    registry: Registry,
    build_ctx: BuildContext,
    /// Compiled in `initialize` (needs the host's buffer size), used in `process`.
    engine: Option<RtEngine>,
    sink: Option<NodeId>,
    sample_rate: f32,
}

/// No automatable parameters yet — host automation routing is task 000065.
#[derive(Params, Default)]
pub struct VstBombParams {}

/// Resource location of a patch shipped inside the bundle, given the dylib's path
/// (`…/Contents/MacOS/lib.dylib` → `…/Contents/Resources/patch.vbp`).
fn resources_patch_path(dylib: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos_dir = dylib.parent()?;
    let contents = macos_dir.parent()?;
    (macos_dir.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.join("Resources").join("patch.vbp"))
}

/// The path of the shared library this code lives in (`dladdr` on unix).
#[cfg(unix)]
fn own_dylib_path() -> Option<std::path::PathBuf> {
    let probe = own_dylib_path as *const std::os::raw::c_void;
    // SAFETY: dladdr only reads the address and fills the (zeroed) info struct;
    // dli_fname, when non-null, is a valid NUL-terminated path owned by the loader.
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(probe, &mut info) != 0 && !info.dli_fname.is_null() {
            let name = std::ffi::CStr::from_ptr(info.dli_fname);
            return Some(std::path::PathBuf::from(
                name.to_string_lossy().into_owned(),
            ));
        }
    }
    None
}

#[cfg(not(unix))]
fn own_dylib_path() -> Option<std::path::PathBuf> {
    None
}

/// The patch this instance should play, in priority order: `VB_PATCH` env override,
/// the patch bundled into the plugin's Resources (via `cargo xtask export`), then
/// the built-in default. Unreadable candidates fall through so the plugin loads.
fn load_patch(registry: &Registry) -> ProjectDoc {
    std::env::var("VB_PATCH")
        .ok()
        .and_then(|path| vb_project::open_path(std::path::Path::new(&path)).ok())
        .or_else(|| {
            own_dylib_path()
                .and_then(|dylib| resources_patch_path(&dylib))
                .and_then(|path| vb_project::open_path(&path).ok())
        })
        .unwrap_or_else(|| default_patch(registry))
}

impl Default for VstBomb {
    fn default() -> Self {
        let registry = Registry::with_builtins();
        let doc = load_patch(&registry);
        Self {
            params: Arc::new(VstBombParams::default()),
            doc,
            registry,
            build_ctx: BuildContext::default(),
            engine: None,
            sink: None,
            sample_rate: 48_000.0,
        }
    }
}

impl Plugin for VstBomb {
    const NAME: &'static str = "VST_bomb";
    const VENDOR: &'static str = "ice1x";
    const URL: &'static str = "https://github.com/ice1x/VST_bomb";
    const EMAIL: &'static str = "tunodnb@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Instrument: no audio input, stereo output.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // Note on/off from the host reach the patch's midi-in / voice-in nodes.
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        let frames = buffer_config.max_buffer_size as usize;
        match engine_from_doc(&self.doc, &self.registry, &self.build_ctx, frames) {
            Ok((engine, sink)) => {
                self.engine = Some(engine);
                self.sink = sink;
                true
            }
            Err(err) => {
                nih_error!("VST_bomb: patch failed to compile: {err}");
                false
            }
        }
    }

    fn reset(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(engine) = &mut self.engine else {
            return ProcessStatus::Error("engine not initialized");
        };
        // Drain the host's note events into the shared mailboxes; midi-in and
        // voice-in processors read them at the coming engine pass. Timing is
        // block-level for now (sample-accurate splitting is later polish).
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { note, velocity, .. } => {
                    route_note(&self.build_ctx, note, velocity, true);
                }
                NoteEvent::NoteOff { note, .. } => {
                    route_note(&self.build_ctx, note, 0.0, false);
                }
                _ => {}
            }
        }
        let frames = buffer.samples();
        engine.process(&vb_core::ProcessContext {
            sample_rate: self.sample_rate,
            frames,
        });

        let output = buffer.as_slice();
        match self
            .sink
            .and_then(|sink| engine.node_output(sink))
            .and_then(|buf| buf.channel(0))
        {
            Some(mono) => {
                // Duplicate the mono sink across every host channel.
                for channel in output.iter_mut() {
                    let n = frames.min(mono.len()).min(channel.len());
                    channel[..n].copy_from_slice(&mono[..n]);
                }
            }
            None => {
                for channel in output.iter_mut() {
                    channel[..frames].fill(0.0);
                }
            }
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for VstBomb {
    const CLAP_ID: &'static str = "com.ice1x.vst-bomb";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Node-graph synthesizer built with VST_bomb");
    const CLAP_MANUAL_URL: Option<&'static str> = Some("https://github.com/ice1x/VST_bomb");
    const CLAP_SUPPORT_URL: Option<&'static str> = Some("https://github.com/ice1x/VST_bomb/issues");
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

/// Routes one host note event into both note mailboxes (mono + poly).
///
/// nih-plug velocities are `[0, 1]`; the mailboxes speak MIDI bytes, so the value is
/// scaled to `0..=127` with rounding and clamping.
fn route_note(ctx: &BuildContext, note: u8, velocity: f32, on: bool) {
    if on {
        let vel = (velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
        ctx.notes.note_on(note, vel);
        ctx.poly.note_on(note, vel);
    } else {
        ctx.notes.note_off(note);
        ctx.poly.note_off(note);
    }
}

nih_export_clap!(VstBomb);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_constructs_with_the_default_patch() {
        let plugin = VstBomb::default();
        assert!(
            !plugin.doc.nodes.is_empty(),
            "default patch has nodes to play"
        );
        assert!(plugin.engine.is_none(), "engine waits for initialize");
    }

    #[test]
    fn host_notes_reach_both_mailboxes() {
        let ctx = BuildContext::default();

        route_note(&ctx, 69, 1.0, true);
        assert_eq!(
            ctx.notes.read(),
            (true, 69, 127),
            "full velocity maps to 127"
        );
        assert_eq!(ctx.poly.read(0), (true, 69, 127));

        route_note(&ctx, 72, 0.5, true);
        let (_, _, vel) = ctx.poly.read(1);
        assert_eq!(vel, 64, "0.5 rounds to 64");

        route_note(&ctx, 69, 0.0, false);
        assert!(
            !ctx.notes.read().0 || ctx.notes.read().1 != 69,
            "mono released or replaced"
        );
        assert!(!ctx.poly.read(0).0, "voice 0 released");
        assert!(ctx.poly.read(1).0, "voice 1 still sounding");

        // Out-of-range velocities clamp instead of wrapping.
        route_note(&ctx, 60, 7.5, true);
        let (_, _, vel) = ctx.poly.read(0);
        assert_eq!(vel, 127);
    }

    #[test]
    fn bundle_resource_path_is_derived_from_the_dylib_location() {
        use std::path::Path;
        let dylib = Path::new("/Library/Audio/vb.clap/Contents/MacOS/vb-plugin");
        assert_eq!(
            resources_patch_path(dylib),
            Some("/Library/Audio/vb.clap/Contents/Resources/patch.vbp".into())
        );
        // A bare dylib outside a bundle has no resource patch.
        assert_eq!(
            resources_patch_path(Path::new("/usr/lib/libfoo.dylib")),
            None
        );
    }

    #[test]
    fn engine_compiles_at_host_buffer_sizes() {
        // The initialize path without the nih-plug harness: same builder, same doc.
        let plugin = VstBomb::default();
        for frames in [64_usize, 256, 1_024, 4_096] {
            let built = engine_from_doc(&plugin.doc, &plugin.registry, &plugin.build_ctx, frames);
            assert!(built.is_ok(), "buffer size {frames}");
        }
    }
}
