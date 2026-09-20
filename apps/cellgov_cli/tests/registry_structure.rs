//! Self-contained structural checks on the title registry and its
//! committed fixtures.
//!
//! The gated set is:
//!
//! - every game title's reference cell, the floor its `system_ver`
//!   derives, and
//! - every declared cell of a title shipped inside the firmware, which
//!   derives no reference.
//!
//! A game title's other rows carry no gate: a manifest declares a cell
//! for free.

#[path = "common/registry.rs"]
mod registry;

use cellgov_compare::BootSummary;
use registry::{boot_anchor_path, firmware_exec_titles, titles, TitleUnderTest, BASE_GAME_VER};

/// Every cell the registry gates on an anchor.
///
/// # Panics
///
/// Panics when the set is empty, since the checks that walk it then
/// cover nothing.
fn gated() -> Vec<TitleUnderTest> {
    let cells: Vec<TitleUnderTest> = titles().into_iter().chain(firmware_exec_titles()).collect();
    assert!(!cells.is_empty(), "the registry gates no cell");
    cells
}

#[test]
fn every_gated_cell_has_a_committed_baseline_or_states_why_not() {
    for t in gated() {
        let p = boot_anchor_path(&t.content_id, &t.reference);
        match (&t.reference.pending, p.is_file()) {
            (None, false) => panic!(
                "{}: no committed baseline for {} at {} -- record it with \
                 `dev record-anchors --title {} --fw {}` on a machine with the dump, or state \
                 `pending = \"<why>\"` on a [[bench.matrix]] row for that cell when something \
                 outside the registry stops it",
                t.short_name,
                t.reference.label(),
                p.display(),
                t.short_name,
                t.reference.fw
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
    for t in gated() {
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

#[test]
fn every_game_titles_gated_cell_is_its_floor_times_its_base_install() {
    let games = titles();
    assert!(!games.is_empty(), "the registry holds no game title");
    for t in &games {
        assert_eq!(
            t.reference.game_ver.as_deref(),
            Some(BASE_GAME_VER),
            "{}: the reference cell is the base install",
            t.short_name
        );
        let p = boot_anchor_path(&t.content_id, &t.reference);
        let tail = format!("fw-{}/{BASE_GAME_VER}/boot_summary.json", t.reference.fw);
        assert!(
            p.to_string_lossy().replace('\\', "/").ends_with(&tail),
            "{}: {} does not end in {tail}",
            t.short_name,
            p.display()
        );
    }
}

#[test]
fn the_system_software_is_declared_at_least_once_and_on_no_game_axis() {
    let cells = firmware_exec_titles();
    assert!(
        !cells.is_empty(),
        "the registry declares no cell for a title shipped inside the firmware"
    );
    for c in &cells {
        assert_eq!(
            c.reference.game_ver, None,
            "{}: a firmware-shipped cell has no game-version axis",
            c.short_name
        );
    }
}
