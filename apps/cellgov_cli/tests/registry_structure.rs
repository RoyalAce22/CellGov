//! Corpus-free structural checks on the title registry and its
//! committed fixtures.

#[path = "common/registry.rs"]
mod registry;

use cellgov_compare::BootSummary;
use registry::{boot_anchor_path, titles, BASE_GAME_VER};

#[test]
fn every_registered_titles_reference_cell_has_a_committed_baseline() {
    for t in titles() {
        let p = boot_anchor_path(&t.content_id, &t.reference);
        match (&t.reference.pending, p.is_file()) {
            (None, false) => panic!(
                "{}: no committed baseline for {} at {} -- record it with \
                 `dev record-anchors --title {}` on a machine with the dump, or state \
                 `pending = \"<why>\"` on the row when something outside the registry \
                 stops it",
                t.short_name,
                t.reference.label(),
                p.display(),
                t.short_name
            ),
            (Some(why), true) => panic!(
                "{}: {} is marked pending ({why}) but {} exists; the cell was measured, \
                 so drop the marker",
                t.short_name,
                t.reference.label(),
                p.display()
            ),
            (None, true) | (Some(_), false) => {}
        }
    }
}

/// The directory path is the only thing that names the cell an anchor
/// is evidence about. An anchor whose recorded identity names another
/// firmware or game version gates the wrong cell and never disagrees
/// with itself.
#[test]
fn every_committed_anchor_names_the_cell_it_is_filed_under() {
    for t in titles() {
        let p = boot_anchor_path(&t.content_id, &t.reference);
        // A cell the manifest declares unmeasurable has nothing filed
        // to check. The sibling test holds the marker honest.
        if t.reference.pending.is_some() && !p.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("{}: read {}: {e}", t.short_name, p.display()));
        let summary: BootSummary = serde_json::from_str(&text).unwrap_or_else(|e| {
            panic!(
                "{}: {} is not a BootSummary: {e}",
                t.short_name,
                p.display()
            )
        });
        assert_eq!(
            summary
                .identity
                .firmware
                .as_ref()
                .map(|f| f.version.as_str()),
            Some(t.reference.fw.as_str()),
            "{}: {} is filed under {} but records another firmware",
            t.short_name,
            p.display(),
            t.reference.label()
        );
        // `GameIdentity::version` spells an update as `update:<ver>`
        // while the path segment is the bare version key.
        let expected: Option<String> = t.reference.game_ver.as_deref().map(|v| {
            if v == BASE_GAME_VER {
                v.to_string()
            } else {
                format!("update:{v}")
            }
        });
        assert_eq!(
            summary.identity.game.as_ref().map(|g| g.version.as_str()),
            expected.as_deref(),
            "{}: {} is filed under {} but records another game version",
            t.short_name,
            p.display(),
            t.reference.label()
        );
    }
}
