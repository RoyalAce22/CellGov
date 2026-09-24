//! Self-contained structural checks for every declared title cell.

#[path = "common/registry.rs"]
mod registry;

use cellgov_compare::{BootSummary, GameIdentity};
use registry::{boot_anchor_path, declared_cells};

#[test]
fn every_declared_matrix_cell_has_a_structurally_valid_anchor_or_reason() {
    let cells = declared_cells();
    assert!(!cells.is_empty(), "the registry declares no matrix cell");
    for cell in cells {
        let path = boot_anchor_path(&cell.content_id, &cell.reference);
        match (&cell.reference.pending, path.is_file()) {
            (Some(_), false) => continue,
            (Some(reason), true) => panic!(
                "{}: {} is pending ({reason}) but {} exists",
                cell.short_name,
                cell.reference.label(),
                path.display()
            ),
            (None, false) => panic!(
                "{}: {} has no anchor at {} and no pending reason",
                cell.short_name,
                cell.reference.label(),
                path.display()
            ),
            (None, true) => {}
        }

        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: read {}: {e}", cell.short_name, path.display()));
        let summary: BootSummary = serde_json::from_str(&text).unwrap_or_else(|e| {
            panic!(
                "{}: {} is not a BootSummary: {e}",
                cell.short_name,
                path.display()
            )
        });
        assert_eq!(
            summary
                .identity
                .firmware
                .as_ref()
                .map(|firmware| firmware.version.as_str()),
            Some(cell.reference.fw.as_str()),
            "{}: {} records another firmware",
            cell.short_name,
            path.display()
        );
        let expected_game = cell
            .reference
            .game_ver
            .as_deref()
            .map(GameIdentity::version_of);
        assert_eq!(
            summary
                .identity
                .game
                .as_ref()
                .map(|game| game.version.as_str()),
            expected_game.as_deref(),
            "{}: {} records another game version",
            cell.short_name,
            path.display()
        );
        assert!(
            !summary.witnesses.is_empty(),
            "{}: {} records no classified witness",
            cell.short_name,
            path.display()
        );
    }
}
