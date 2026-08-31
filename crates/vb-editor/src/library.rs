//! Library browser: pure list / search / tag-filter logic for the editor's library panel
//! (backlog 000097). The egui side panel renders what this returns — a text query, a set of
//! tag chips, and the list of matching items — and inserting a chosen item onto the canvas
//! is the app's job. Keeping the interaction as a framework-free view-model means it is unit
//! tested here without a display.
//!
//! The item source (the drevo-backed library) is deliberately abstracted away: a
//! [`LibraryBrowser`] filters whatever [`LibraryItem`]s it is given, so the same logic serves
//! an in-memory list today and the real library once that path is wired.

/// One browsable library entry: a name plus its tags. `name` is the handle the app uses to
/// insert the item (a macro-node / preset) onto the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryItem {
    /// Human-readable name, also the insertion handle.
    pub name: String,
    /// Tags for faceted filtering (e.g. `["bass", "analog"]`).
    pub tags: Vec<String>,
}

impl LibraryItem {
    /// Creates an item from a name and tags.
    pub fn new(name: impl Into<String>, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            name: name.into(),
            tags: tags.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether this item carries `tag` (case-insensitive).
    fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
    }
}

/// The library panel's state: the full item set, the search query, and the selected tag
/// facets. `visible()` is what the panel lists.
#[derive(Debug, Clone, Default)]
pub struct LibraryBrowser {
    items: Vec<LibraryItem>,
    query: String,
    selected_tags: Vec<String>,
}

impl LibraryBrowser {
    /// Creates a browser over `items`.
    #[must_use]
    pub fn new(items: Vec<LibraryItem>) -> Self {
        Self {
            items,
            query: String::new(),
            selected_tags: Vec::new(),
        }
    }

    /// Replaces the item set (e.g. after the library reloads), keeping the current filters.
    pub fn set_items(&mut self, items: Vec<LibraryItem>) {
        self.items = items;
    }

    /// The current search query.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Sets the case-insensitive name search query.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// The currently selected tag facets.
    #[must_use]
    pub fn selected_tags(&self) -> &[String] {
        &self.selected_tags
    }

    /// Whether `tag` is currently an active facet (case-insensitive).
    #[must_use]
    pub fn is_tag_selected(&self, tag: &str) -> bool {
        self.selected_tags
            .iter()
            .any(|t| t.eq_ignore_ascii_case(tag))
    }

    /// Toggles a tag facet on or off.
    pub fn toggle_tag(&mut self, tag: &str) {
        if let Some(pos) = self
            .selected_tags
            .iter()
            .position(|t| t.eq_ignore_ascii_case(tag))
        {
            self.selected_tags.remove(pos);
        } else {
            self.selected_tags.push(tag.to_owned());
        }
    }

    /// Clears the query and every selected tag.
    pub fn clear_filters(&mut self) {
        self.query.clear();
        self.selected_tags.clear();
    }

    /// Every distinct tag across all items, case-insensitively de-duplicated and sorted —
    /// the chips the panel offers. The first-seen spelling of each tag is kept.
    #[must_use]
    pub fn all_tags(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for item in &self.items {
            for tag in &item.tags {
                if !seen.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                    seen.push(tag.clone());
                }
            }
        }
        seen.sort_by_key(|t| t.to_lowercase());
        seen
    }

    /// The items matching the current filters: name contains the query (case-insensitive)
    /// **and** the item carries every selected tag (AND across facets). Sorted by name.
    #[must_use]
    pub fn visible(&self) -> Vec<&LibraryItem> {
        let needle = self.query.trim().to_lowercase();
        let mut out: Vec<&LibraryItem> = self
            .items
            .iter()
            .filter(|item| {
                let name_ok = needle.is_empty() || item.name.to_lowercase().contains(&needle);
                let tags_ok = self.selected_tags.iter().all(|tag| item.has_tag(tag));
                name_ok && tags_ok
            })
            .collect();
        out.sort_by_key(|item| item.name.to_lowercase());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> LibraryBrowser {
        LibraryBrowser::new(vec![
            LibraryItem::new("Warm Bass", ["bass", "analog"]),
            LibraryItem::new("Acid Bass", ["bass", "acid"]),
            LibraryItem::new("Bright Lead", ["lead", "bright"]),
            LibraryItem::new("Analog Pad", ["pad", "Analog"]),
        ])
    }

    fn names(items: &[&LibraryItem]) -> Vec<String> {
        items.iter().map(|i| i.name.clone()).collect()
    }

    #[test]
    fn no_filters_lists_everything_sorted_by_name() {
        let lib = library();
        assert_eq!(
            names(&lib.visible()),
            vec!["Acid Bass", "Analog Pad", "Bright Lead", "Warm Bass"]
        );
    }

    #[test]
    fn a_query_matches_names_case_insensitively() {
        let mut lib = library();
        lib.set_query("bass");
        assert_eq!(names(&lib.visible()), vec!["Acid Bass", "Warm Bass"]);
        lib.set_query("BRIGHT");
        assert_eq!(names(&lib.visible()), vec!["Bright Lead"]);
    }

    #[test]
    fn selecting_a_tag_narrows_to_items_with_that_tag() {
        let mut lib = library();
        lib.toggle_tag("bass");
        assert_eq!(names(&lib.visible()), vec!["Acid Bass", "Warm Bass"]);
    }

    #[test]
    fn multiple_tags_are_anded_together() {
        let mut lib = library();
        lib.toggle_tag("bass");
        lib.toggle_tag("analog");
        // Only "Warm Bass" carries both bass and analog.
        assert_eq!(names(&lib.visible()), vec!["Warm Bass"]);
    }

    #[test]
    fn query_and_tags_combine() {
        let mut lib = library();
        lib.toggle_tag("bass");
        lib.set_query("acid");
        assert_eq!(names(&lib.visible()), vec!["Acid Bass"]);
    }

    #[test]
    fn tag_matching_is_case_insensitive() {
        let mut lib = library();
        // "Analog Pad" is tagged "Analog", the query capitalisation should not matter.
        lib.toggle_tag("ANALOG");
        let visible = names(&lib.visible());
        assert!(visible.contains(&"Analog Pad".to_owned()));
        assert!(visible.contains(&"Warm Bass".to_owned()));
    }

    #[test]
    fn all_tags_are_deduped_case_insensitively_and_sorted() {
        let lib = library();
        // "analog" and "Analog" collapse to one chip; sorted alphabetically (lowercased).
        assert_eq!(
            lib.all_tags(),
            vec!["acid", "analog", "bass", "bright", "lead", "pad"]
        );
    }

    #[test]
    fn toggle_tag_is_reversible_and_reported() {
        let mut lib = library();
        assert!(!lib.is_tag_selected("bass"));
        lib.toggle_tag("bass");
        assert!(lib.is_tag_selected("bass"));
        assert_eq!(lib.selected_tags(), &["bass".to_owned()]);
        lib.toggle_tag("bass");
        assert!(!lib.is_tag_selected("bass"));
        assert!(lib.selected_tags().is_empty());
    }

    #[test]
    fn clear_filters_resets_query_and_tags() {
        let mut lib = library();
        lib.set_query("bass");
        lib.toggle_tag("analog");
        lib.clear_filters();
        assert_eq!(lib.query(), "");
        assert!(lib.selected_tags().is_empty());
        assert_eq!(lib.visible().len(), 4, "everything is visible again");
    }

    #[test]
    fn a_filter_that_matches_nothing_yields_an_empty_list() {
        let mut lib = library();
        lib.set_query("nonexistent");
        assert!(lib.visible().is_empty());
    }

    #[test]
    fn set_items_swaps_the_source_keeping_filters() {
        let mut lib = library();
        lib.toggle_tag("bass");
        lib.set_items(vec![
            LibraryItem::new("Sub Bass", ["bass"]),
            LibraryItem::new("Airy Pad", ["pad"]),
        ]);
        // The bass filter still applies to the new set.
        assert_eq!(names(&lib.visible()), vec!["Sub Bass"]);
    }
}
