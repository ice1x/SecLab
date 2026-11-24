//! Where the audio-settings window lives.
//!
//! It used to be a collapsed floating window parked at egui's default spot, the top-left
//! corner, so its title bar always covered the editor's sidebar / panel header (visible in
//! every screenshot). Now the status strip's device readout is a toggle button, and the
//! window only exists while it is open, pinned just above the status strip.

/// The status-strip device readout ("<device> @ <rate> Hz / <block> frames"), as a button
/// that opens and closes the audio-settings window.
pub fn device_button(ui: &mut egui::Ui, open: &mut bool, readout: &str) -> egui::Response {
    let response = ui
        .selectable_label(*open, format!("\u{2699} {readout}"))
        .on_hover_text("audio device, sample rate, buffer size and MIDI input");
    if response.clicked() {
        *open = !*open;
    }
    response
}

/// Shows the audio-settings window while `open` is set (its × button clears it), opening
/// upwards from the top-left of `status` (the status strip). Returns `None` — nothing
/// drawn — once closed (after egui's short fade-out).
pub fn settings_window<R>(
    ctx: &egui::Context,
    open: &mut bool,
    status: egui::Rect,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<egui::InnerResponse<Option<R>>> {
    egui::Window::new(TITLE)
        .open(open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .default_pos(status.left_top() + egui::vec2(8.0, -8.0))
        .show(ctx, add_contents)
}

/// The window's title (and so its egui id).
pub const TITLE: &str = "audio";

#[cfg(test)]
mod tests {
    use super::*;

    fn input(ctx: &egui::Context, events: Vec<egui::Event>) -> egui::RawInput {
        // Advance the clock so egui's open/close fades finish.
        let time = ctx.input(|i| i.time) + 0.1;
        egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        }
    }

    fn click_at(pos: egui::Pos2) -> Vec<Vec<egui::Event>> {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![
            vec![egui::Event::PointerMoved(pos)],
            vec![button(true)],
            vec![button(false)],
        ]
    }

    /// One frame: a status strip with the device button, plus the window.
    fn frame(ctx: &egui::Context, open: &mut bool, events: Vec<egui::Event>) -> FrameOut {
        let mut out = FrameOut {
            clicked: false,
            status_top: 0.0,
            button: egui::Rect::NOTHING,
            window: None,
        };
        let _ = ctx.run(input(ctx, events), |ctx| {
            let status = egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let b = device_button(ui, open, "Speakers @ 48000 Hz / 256 frames");
                    out.clicked = b.clicked();
                    out.button = b.rect;
                });
            });
            out.status_top = status.response.rect.top();
            egui::CentralPanel::default().show(ctx, |_| {});
            out.window = settings_window(ctx, open, status.response.rect, |ui| ui.label("device"))
                .map(|r| r.response.rect);
        });
        out
    }

    struct FrameOut {
        clicked: bool,
        status_top: f32,
        button: egui::Rect,
        window: Option<egui::Rect>,
    }

    #[test]
    fn the_window_is_hidden_until_the_device_readout_is_clicked() {
        let ctx = egui::Context::default();
        let mut open = false;
        let first = frame(&ctx, &mut open, vec![]);
        assert!(
            first.window.is_none(),
            "nothing floats over the editor by default"
        );

        let mut clicked = false;
        for events in click_at(first.button.center()) {
            clicked |= frame(&ctx, &mut open, events).clicked;
        }
        assert!(clicked && open);
        let shown = frame(&ctx, &mut open, vec![]);
        let window = shown.window.expect("open after the click");
        // Pinned above the status strip, clear of the editor's top-left corner.
        assert!(
            window.bottom() <= shown.status_top + 1.0,
            "{window:?} vs {}",
            shown.status_top
        );
        assert!(window.top() > 100.0, "{window:?}");

        // A second click closes it again.
        for events in click_at(shown.button.center()) {
            frame(&ctx, &mut open, events);
        }
        assert!(!open);
        let mut last = None;
        for _ in 0..10 {
            last = frame(&ctx, &mut open, vec![]).window;
        }
        assert!(last.is_none(), "gone once the fade-out ends");
    }
}
