//! Guards the root CHANGELOG against rot: it must keep an `[Unreleased]` section and must
//! document the current on-disk project format version. Because the format version lives
//! in code (`vb_project::FORMAT_VERSION`), bumping the schema without noting it in the
//! changelog fails here — a reminder to record the format change where releases are
//! tracked (backlog 000091).

use vb_project::FORMAT_VERSION;

const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

#[test]
fn changelog_has_an_unreleased_section() {
    assert!(
        CHANGELOG.contains("## [Unreleased]"),
        "CHANGELOG.md must keep an [Unreleased] section for in-progress work"
    );
}

#[test]
fn changelog_documents_the_current_format_version() {
    let needle = format!("format version {FORMAT_VERSION}");
    assert!(
        CHANGELOG.contains(&needle),
        "CHANGELOG.md must mention the current project '{needle}' — bump the note when \
         vb_project::FORMAT_VERSION changes"
    );
}
