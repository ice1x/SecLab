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

impl Default for VstBomb {
    fn default() -> Self {
        let registry = Registry::with_builtins();
        // A patch file from the environment wins; anything unreadable falls back to
        // the built-in patch so the plugin always loads.
        let doc = std::env::var("VB_PATCH")
            .ok()
            .and_then(|path| vb_project::open_path(std::path::Path::new(&path)).ok())
            .unwrap_or_else(|| default_patch(&registry));
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
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(engine) = &mut self.engine else {
            return ProcessStatus::Error("engine not initialized");
        };
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
    fn engine_compiles_at_host_buffer_sizes() {
        // The initialize path without the nih-plug harness: same builder, same doc.
        let plugin = VstBomb::default();
        for frames in [64_usize, 256, 1_024, 4_096] {
            let built = engine_from_doc(&plugin.doc, &plugin.registry, &plugin.build_ctx, frames);
            assert!(built.is_ok(), "buffer size {frames}");
        }
    }
}
