//! The firmware page: which titles reach it, one row per declared
//! cell, and its coverage line.

use cellgov_compare::BootOutcome;

use super::super::load::load_title;
use super::super::test_fixtures::*;
use super::*;

/// Every table row of `body`, the header and separator excluded.
fn rows(body: &str) -> Vec<&str> {
    body.lines().filter(|l| l.starts_with("| [")).collect()
}

#[test]
fn a_game_title_contributes_no_row() {
    let game = title("NPAA00001", "Game", 2007, "Studio");
    let fixtures = Fixtures::new("fw-page-game");
    fixtures.write_cross("NPAA00001", &reference_key(), &converged(0));
    let body = render(&[load_title(&game, fixtures.path()).unwrap()]);
    assert!(rows(&body).is_empty(), "{body}");
    assert!(!body.contains("NPAA00001"), "{body}");
    assert!(
        body.contains(
            "Coverage: 0 firmware-shipped title(s), 0 firmware(s), 0 declared cell(s), 0 recorded."
        ),
        "{body}"
    );
}

#[test]
fn a_firmware_shipped_title_renders_one_row_per_declared_cell_in_declaration_order() {
    let t = firmware_exec_title("VSHTEST", "System Software", &["4.93", "1.50", "3.55"]);
    let fixtures = Fixtures::new("fw-page-rows");
    fixtures.write_anchor(
        "VSHTEST",
        &cell_key("1.50", None),
        &boot(BootOutcome::MaxSteps, 389_859),
    );
    let body = render(&[load_title(&t, fixtures.path()).unwrap()]);
    let rows = rows(&body);
    assert_eq!(rows.len(), 3, "{body}");
    assert!(
        rows[0].starts_with("| [VSHTEST](titles/VSHTEST.md) | System Software | fw 4.93 |"),
        "{}",
        rows[0]
    );
    assert!(
        rows[0].ends_with("| -- | -- | -- | -- | -- |"),
        "{}",
        rows[0]
    );
    assert!(
        rows[1].contains("| fw 1.50 | ProcessExit -> MaxSteps | 389,859 | 99,803,904 | -- | -- |"),
        "{}",
        rows[1]
    );
    assert!(rows[2].contains("| fw 3.55 |"), "{}", rows[2]);
    assert!(
        body.contains(
            "Coverage: 1 firmware-shipped title(s), 3 firmware(s), 3 declared cell(s), 1 recorded."
        ),
        "{body}"
    );
}

#[test]
fn a_firmware_shipped_title_declaring_no_cells_is_counted_and_renders_no_row() {
    let t = firmware_exec_title("VSHNONE", "Bare", &[]);
    let fixtures = Fixtures::new("fw-page-none");
    let body = render(&[load_title(&t, fixtures.path()).unwrap()]);
    assert!(rows(&body).is_empty(), "{body}");
    assert!(
        body.contains(
            "Coverage: 1 firmware-shipped title(s), 0 firmware(s), 0 declared cell(s), 0 recorded."
        ),
        "{body}"
    );
}

#[test]
fn a_converged_cell_renders_its_verdict_columns() {
    let t = firmware_exec_title("VSHOK", "Converged", &["4.93"]);
    let fixtures = Fixtures::new("fw-page-converged");
    fixtures.write_cross("VSHOK", &cell_key("4.93", None), &converged(12));
    let body = render(&[load_title(&t, fixtures.path()).unwrap()]);
    assert!(
        rows(&body)[0].ends_with("| fw 4.93 | -- | -- | -- | Yes | 12 non-semantic |"),
        "{body}"
    );
}

#[test]
fn the_page_is_byte_identical_across_two_renders_and_leaves_no_token_unsubstituted() {
    let t = firmware_exec_title("VSHDET", "Deterministic", &["4.93"]);
    let fixtures = Fixtures::new("fw-page-det");
    let docs = [load_title(&t, fixtures.path()).unwrap()];
    let body = render(&docs);
    assert_eq!(body, render(&docs));
    assert!(!body.contains("{{"), "unsubstituted token in {body}");
    assert!(body.contains("[title matrix](titles.md)"), "{body}");
}
