//! Self-screenshot mode for the docs and the release page.
//!
//! `SECLAB_SCREENSHOT=<out.png> seclab-standalone` opens the editor (optionally on the patch in
//! `SECLAB_SCREENSHOT_PATCH`), lets the layout settle for [`SETTLE_FRAMES`] frames, asks eframe
//! for a screenshot of its own window, writes it as a PNG and closes the app. No OS screen
//! recording permission is involved: the pixels come from the app's own framebuffer.

use std::path::{Path, PathBuf};

/// Output PNG path; setting it turns the mode on.
pub const ENV_OUT: &str = "SECLAB_SCREENSHOT";
/// Optional patch (`.ron`/`.vbp`) to open before the screenshot.
pub const ENV_PATCH: &str = "SECLAB_SCREENSHOT_PATCH";
/// Optional surface to show: `panel` or `blocks` (the default).
pub const ENV_VIEW: &str = "SECLAB_SCREENSHOT_VIEW";

/// The editor surface named by [`ENV_VIEW`].
///
/// # Errors
/// On an unknown name.
pub fn parse_view(name: &str) -> Result<seclab_editor::ViewMode, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "" | "blocks" | "structure" => Ok(seclab_editor::ViewMode::Structure),
        "panel" => Ok(seclab_editor::ViewMode::Panel),
        other => Err(format!("{ENV_VIEW}={other}: expected `blocks` or `panel`")),
    }
}

/// Frames to wait before the screenshot, so fonts, layout and the canvas auto-fit settle.
pub const SETTLE_FRAMES: u32 = 30;

/// What the app should do this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Keep rendering.
    Wait,
    /// Send `ViewportCommand::Screenshot` now.
    Request,
    /// The screenshot was handled (`Ok` = written); close the app.
    Done(Result<PathBuf, String>),
}

/// Drives one screenshot: settle → request → save.
#[derive(Debug)]
pub struct Screenshotter {
    out: PathBuf,
    frames: u32,
    requested: bool,
    done: bool,
}

impl Screenshotter {
    /// A screenshotter writing to `out`.
    #[must_use]
    pub fn new(out: impl Into<PathBuf>) -> Self {
        Self {
            out: out.into(),
            frames: 0,
            requested: false,
            done: false,
        }
    }

    /// The mode, when [`ENV_OUT`] is set to a non-empty path.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        std::env::var_os(ENV_OUT)
            .filter(|v| !v.is_empty())
            .map(Self::new)
    }

    /// Called once per frame with that frame's input events.
    pub fn on_frame(&mut self, events: &[egui::Event]) -> Step {
        if self.done {
            return Step::Wait;
        }
        if self.requested {
            for e in events {
                if let egui::Event::Screenshot { image, .. } = e {
                    self.done = true;
                    return Step::Done(save_png(image, &self.out).map(|()| self.out.clone()));
                }
            }
            return Step::Wait;
        }
        self.frames += 1;
        if self.frames >= SETTLE_FRAMES {
            self.requested = true;
            Step::Request
        } else {
            Step::Wait
        }
    }
}

/// Writes `image` as an RGBA PNG (creating the parent folder).
///
/// # Errors
/// When the folder or file cannot be written.
pub fn save_png(image: &egui::ColorImage, path: &Path) -> Result<(), String> {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let buf = image::RgbaImage::from_raw(
        u32::try_from(w).map_err(|e| e.to_string())?,
        u32::try_from(h).map_err(|e| e.to_string())?,
        rgba,
    )
    .ok_or("screenshot size does not match its pixels")?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    buf.save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("seclab-shot-{name}-{}", std::process::id()))
    }

    fn shot_event(image: egui::ColorImage) -> egui::Event {
        egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::default(),
            image: Arc::new(image),
        }
    }

    fn red_green() -> egui::ColorImage {
        egui::ColorImage {
            size: [2, 1],
            pixels: vec![egui::Color32::RED, egui::Color32::GREEN],
        }
    }

    #[test]
    fn it_waits_for_the_layout_to_settle_then_requests_once() {
        let mut s = Screenshotter::new(temp("settle").join("a.png"));
        for _ in 1..SETTLE_FRAMES {
            assert_eq!(s.on_frame(&[]), Step::Wait);
        }
        assert_eq!(s.on_frame(&[]), Step::Request);
        // No second request while waiting for the reply.
        for _ in 0..5 {
            assert_eq!(s.on_frame(&[]), Step::Wait);
        }
    }

    #[test]
    fn a_screenshot_before_the_request_is_ignored() {
        let dir = temp("early");
        let mut s = Screenshotter::new(dir.join("a.png"));
        assert_eq!(s.on_frame(&[shot_event(red_green())]), Step::Wait);
        assert!(!dir.exists());
    }

    #[test]
    fn the_reply_is_saved_as_png_and_finishes_the_run() {
        let dir = temp("save");
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join("nested").join("editor.png");
        let mut s = Screenshotter::new(&out);
        while s.on_frame(&[]) != Step::Request {}
        assert_eq!(
            s.on_frame(&[shot_event(red_green())]),
            Step::Done(Ok(out.clone()))
        );
        let back = image::open(&out).unwrap().to_rgba8();
        assert_eq!(back.dimensions(), (2, 1));
        assert_eq!(back.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(back.get_pixel(1, 0).0, [0, 255, 0, 255]);
        // Afterwards nothing more happens.
        assert_eq!(s.on_frame(&[shot_event(red_green())]), Step::Wait);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_failure_is_reported_not_panicked() {
        let blocker = temp("blocker");
        std::fs::write(&blocker, b"a file, not a folder").unwrap();
        let mut s = Screenshotter::new(blocker.join("x.png"));
        while s.on_frame(&[]) != Step::Request {}
        match s.on_frame(&[shot_event(red_green())]) {
            Step::Done(Err(e)) => assert!(e.contains("blocker"), "{e}"),
            other => panic!("expected an error, got {other:?}"),
        }
        let _ = std::fs::remove_file(&blocker);
    }

    #[test]
    fn the_view_is_blocks_by_default_or_panel_on_request() {
        use seclab_editor::ViewMode;
        assert_eq!(parse_view(""), Ok(ViewMode::Structure));
        assert_eq!(parse_view("blocks"), Ok(ViewMode::Structure));
        assert_eq!(parse_view(" Panel "), Ok(ViewMode::Panel));
        assert!(parse_view("knobs").unwrap_err().contains("knobs"));
    }

    #[test]
    fn a_mismatched_image_is_an_error() {
        let bad = egui::ColorImage {
            size: [3, 3],
            pixels: vec![egui::Color32::RED],
        };
        assert!(save_png(&bad, &temp("bad.png")).is_err());
    }
}
