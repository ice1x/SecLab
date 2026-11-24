//! The README's UI tour GIF (backlog 000089).
//!
//! `cargo xtask readme-tour` opens a few factory patches in the standalone's self-screenshot
//! mode (`SECLAB_SCREENSHOT`, see `crates/seclab-standalone/src/screenshot.rs`), one PNG per
//! [`Shot`], and encodes them with [`encode_gif`] into a looping slideshow at
//! `docs/images/tour.gif`. The pixels come from the app's own framebuffer, so no screen
//! recording is involved and the result is reproducible.

use image::RgbaImage;

/// One frame of the tour: a factory patch (file name in `examples/`) on one editor surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shot {
    /// File name in `examples/`.
    pub patch: &'static str,
    /// `panel` or `blocks` (`SECLAB_SCREENSHOT_VIEW`).
    pub view: &'static str,
}

/// The tour, in order.
pub const TOUR: &[Shot] = &[
    Shot {
        patch: "minitaur.ron",
        view: "panel",
    },
    Shot {
        patch: "first_synth.ron",
        view: "blocks",
    },
    Shot {
        patch: "juno60.ron",
        view: "panel",
    },
    Shot {
        patch: "vocoder_choir.ron",
        view: "blocks",
    },
    Shot {
        patch: "ms20.ron",
        view: "panel",
    },
    Shot {
        patch: "west_coast_fold.ron",
        view: "blocks",
    },
];

/// Width of the GIF in pixels; frames are scaled down to it, keeping their aspect ratio.
pub const GIF_WIDTH: u32 = 960;

/// How long each frame stays on screen.
pub const FRAME_DELAY_MS: u32 = 2500;

/// Encodes `frames` as an endlessly looping GIF, each scaled to `width` pixels wide and shown
/// for `delay_ms`.
///
/// # Errors
/// When there are no frames, the frames differ in size (a GIF has one canvas), or encoding
/// fails.
pub fn encode_gif(frames: &[RgbaImage], width: u32, delay_ms: u32) -> Result<Vec<u8>, String> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::imageops::{resize, FilterType};
    use image::{Delay, Frame};

    let first = frames.first().ok_or("no frames to encode")?;
    let (w, h) = first.dimensions();
    if let Some(odd) = frames.iter().find(|f| f.dimensions() != (w, h)) {
        return Err(format!(
            "frame size {:?} differs from the first frame's {:?}; a GIF has one canvas",
            odd.dimensions(),
            (w, h)
        ));
    }
    let out_w = width.min(w).max(1);
    let out_h = (u64::from(h) * u64::from(out_w) / u64::from(w)).max(1) as u32;
    let delay = Delay::from_numer_denom_ms(delay_ms, 1);

    let mut gif = Vec::new();
    {
        let mut encoder = GifEncoder::new_with_speed(&mut gif, 10);
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        let scaled = frames.iter().map(|f| {
            let f = if out_w == w {
                f.clone()
            } else {
                resize(f, out_w, out_h, FilterType::Lanczos3)
            };
            Frame::from_parts(f, 0, 0, delay)
        });
        encoder.encode_frames(scaled).map_err(|e| e.to_string())?;
    }
    Ok(gif)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifDecoder;
    use image::{AnimationDecoder, Rgba};
    use std::path::Path;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255]))
    }

    fn decode(gif: &[u8]) -> Vec<image::Frame> {
        GifDecoder::new(std::io::Cursor::new(gif))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap()
    }

    #[test]
    fn frames_are_scaled_to_the_width_keeping_the_aspect_ratio() {
        let frames = [solid(400, 250, [255, 0, 0]), solid(400, 250, [0, 0, 255])];
        let gif = encode_gif(&frames, 200, 1500).unwrap();
        let decoded = decode(&gif);
        assert_eq!(decoded.len(), 2);
        for f in &decoded {
            assert_eq!(f.buffer().dimensions(), (200, 125));
            let (num, den) = f.delay().numer_denom_ms();
            assert_eq!(num / den, 1500);
        }
        // The colours survive quantization.
        let red = decoded[0].buffer().get_pixel(100, 60);
        assert!(red[0] > 200 && red[2] < 50, "{red:?}");
        let blue = decoded[1].buffer().get_pixel(100, 60);
        assert!(blue[2] > 200 && blue[0] < 50, "{blue:?}");
    }

    #[test]
    fn the_gif_loops_forever() {
        let gif = encode_gif(&[solid(10, 10, [9, 9, 9])], 10, 100).unwrap();
        assert!(gif.starts_with(b"GIF89a"));
        // The NETSCAPE2.0 application extension with a loop count of 0 = infinite.
        let at = gif
            .windows(11)
            .position(|w| w == b"NETSCAPE2.0")
            .expect("looping extension");
        assert_eq!(&gif[at + 11..at + 15], &[3, 1, 0, 0]);
    }

    #[test]
    fn a_frame_narrower_than_the_width_is_not_upscaled() {
        let gif = encode_gif(&[solid(100, 50, [0, 255, 0])], 960, 100).unwrap();
        assert_eq!(decode(&gif)[0].buffer().dimensions(), (100, 50));
    }

    #[test]
    fn no_frames_or_mixed_sizes_are_errors() {
        assert!(encode_gif(&[], 100, 100).unwrap_err().contains("no frames"));
        let err = encode_gif(&[solid(10, 10, [0; 3]), solid(20, 10, [0; 3])], 10, 100).unwrap_err();
        assert!(err.contains("size"), "{err}");
    }

    #[test]
    fn every_tour_shot_is_a_factory_patch_on_a_known_view() {
        assert!(TOUR.len() >= 4, "a tour, not a single picture");
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
        for shot in TOUR {
            assert!(examples.join(shot.patch).is_file(), "{}", shot.patch);
            assert!(matches!(shot.view, "panel" | "blocks"), "{}", shot.view);
        }
        assert!(TOUR.iter().any(|s| s.view == "panel"));
        assert!(TOUR.iter().any(|s| s.view == "blocks"));
    }

    #[test]
    fn the_readme_shows_the_committed_tour_gif() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
        assert!(readme.contains("](docs/images/tour.gif)"));
        let gif = std::fs::read(root.join("docs/images/tour.gif")).unwrap();
        let frames = decode(&gif);
        assert_eq!(frames.len(), TOUR.len(), "re-run `cargo xtask readme-tour`");
        assert_eq!(frames[0].buffer().width(), GIF_WIDTH);
    }
}
