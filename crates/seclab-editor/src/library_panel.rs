//! egui layout for the library browser side panel (backlog 000097).
//!
//! A thin rendering shell over the tested [`crate::library::LibraryBrowser`] view-model:
//! a search field, a row of toggleable tag chips, and the filtered result list. All the
//! filtering/search logic lives (and is unit-tested) in `LibraryBrowser`; this file only
//! turns that state into widgets and reports which item the user chose to insert. It is the
//! visual sketch — the exact styling and where it sits in the editor layout are for the app
//! to finalize.

use egui::Ui;

use crate::library::LibraryBrowser;

/// Renders the library browser into `ui`, driving `browser`'s query and tag facets from the
/// widgets. Returns `Some(name)` when the user clicks an item to insert it (the caller wires
/// that to importing the corresponding macro-node onto the canvas).
pub fn library_browser_ui(ui: &mut Ui, browser: &mut LibraryBrowser) -> Option<String> {
    ui.label("library");

    // Search field — case-insensitive name match, applied as it changes.
    let mut query = browser.query().to_owned();
    if ui
        .add(egui::TextEdit::singleline(&mut query).hint_text("search…"))
        .changed()
    {
        browser.set_query(query);
    }

    // Tag facet chips: click to AND a tag into the filter, click again to drop it.
    let tags = browser.all_tags();
    if !tags.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for tag in &tags {
                let selected = browser.is_tag_selected(tag);
                if ui.selectable_label(selected, tag).clicked() {
                    browser.toggle_tag(tag);
                }
            }
        });
    }

    let has_filters = !browser.query().is_empty() || !browser.selected_tags().is_empty();
    if has_filters && ui.button("clear filters").clicked() {
        browser.clear_filters();
    }

    ui.separator();

    // Result list: each match is a button that requests its insertion.
    let mut chosen = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        let visible = browser.visible();
        if visible.is_empty() {
            ui.weak("no matches");
        } else {
            for item in visible {
                let label = if item.tags.is_empty() {
                    item.name.clone()
                } else {
                    format!("{}  ·  {}", item.name, item.tags.join(", "))
                };
                if ui.button(label).clicked() {
                    chosen = Some(item.name.clone());
                }
            }
        }
    });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::LibraryItem;

    fn browser() -> LibraryBrowser {
        LibraryBrowser::new(vec![
            LibraryItem::new("Warm Bass", ["bass", "analog"]),
            LibraryItem::new("Bright Lead", ["lead"]),
        ])
    }

    /// Runs `library_browser_ui` once inside a headless egui context — no window, no display.
    /// This is a smoke test: it proves the layout code builds a frame without panicking
    /// (visual correctness is verified by a human). Returns whatever the panel returned.
    fn run_once(browser: &mut LibraryBrowser) -> Option<String> {
        let ctx = egui::Context::default();
        let mut chosen = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                chosen = library_browser_ui(ui, browser);
            });
        });
        chosen
    }

    #[test]
    fn the_panel_renders_without_panicking() {
        let mut b = browser();
        // No interaction this frame → nothing chosen, but the frame must build cleanly.
        assert!(run_once(&mut b).is_none());
    }

    #[test]
    fn it_renders_with_active_filters_and_empty_results() {
        let mut b = browser();
        b.set_query("nothing matches this");
        assert!(b.visible().is_empty());
        // The "no matches" branch and the clear-filters button must also render cleanly.
        assert!(run_once(&mut b).is_none());
    }

    #[test]
    fn it_renders_an_empty_library() {
        let mut b = LibraryBrowser::default();
        assert!(run_once(&mut b).is_none());
    }
}
