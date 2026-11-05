use std::path::{Path, PathBuf};

use super::entry::run_inner;
use super::error::FuzzCliError;
use super::ppu_reference::render_ppu_reference;

use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::{try_parse, Command, DevCommand, FuzzArgs, FuzzCommand};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/cellgov_fuzz/tests/fixtures/ppu_reference")
}

fn parse(dir: &Path) -> FuzzArgs {
    let argv = [
        "cellgov",
        "dev",
        "fuzz",
        "ppu-reference",
        dir.to_str().expect("utf-8 path"),
    ];
    let cli = try_parse(&argv.iter().map(ToString::to_string).collect::<Vec<_>>())
        .expect("ppu-reference parses");
    let Command::Dev(DevCommand::Fuzz(fuzz)) = cli.command else {
        panic!("fuzz must remain a dev command");
    };
    assert!(matches!(fuzz.command, FuzzCommand::PpuReference(_)));
    fuzz
}

#[test]
fn the_committed_directory_replays_clean() {
    let dir = fixtures();
    assert_eq!(
        run_inner(&parse(&dir)).expect("runs"),
        CommandExitCode::SUCCESS
    );
    let campaign =
        cellgov_fuzz::ppu_reference::run_reference_directory(&dir).expect("the directory reads");
    let text = render_ppu_reference(&campaign);
    assert!(
        text.contains(
            "fuzz ppu-reference: addi.json li-r3-7 unit=instruction:addi match differences=[] \
             paths=agree"
        ),
        "{text}"
    );
    assert!(text.contains("missing=0 duplicated=0 unowned=0 stale_pending=0 unknown_pending=0"));
}

#[test]
fn a_differing_file_or_a_gap_fails_and_is_named() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_ppu_reference");
    let dir: &Path = &scratch;
    let fixture = std::fs::read_to_string(fixtures().join("addi.json")).expect("reads");
    let mut wrong: serde_json::Value = serde_json::from_str(&fixture).expect("JSON");
    wrong["vectors"][0]["expected"]["state"]["pc"]["value"] = 8.into();
    std::fs::write(dir.join("addi.json"), wrong.to_string()).expect("writes");
    let failed = run_inner(&parse(dir)).expect("runs");
    assert_eq!(failed, CommandExitCode::new(exit_codes::FAILED));
    let campaign =
        cellgov_fuzz::ppu_reference::run_reference_directory(dir).expect("the directory reads");
    let text = render_ppu_reference(&campaign);
    assert!(
        text.contains("addi.json li-r3-7 unit=instruction:addi differs differences=[StatePc]"),
        "{text}"
    );
    assert!(text.contains("fuzz ppu-reference: missing facility:time_base\n"));
}

/// Copies the committed reference files into `dir`, and the pending list
/// when `pending` is set.
fn copy_committed(dir: &Path, pending: bool) {
    for entry in std::fs::read_dir(fixtures()).expect("the fixtures list") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().expect("a name").to_owned();
        let is_json = path.extension().is_some_and(|ext| ext == "json");
        let is_pending = name == cellgov_fuzz::ppu_reference::PPU_REFERENCE_PENDING_FILE;
        if is_json || (pending && is_pending) {
            std::fs::copy(&path, dir.join(&name)).expect("copies");
        }
    }
}

#[test]
fn every_vector_matching_still_fails_when_a_unit_has_no_file_or_pending_entry() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_ppu_reference_gap");
    let dir: &Path = &scratch;
    copy_committed(dir, false);
    assert_eq!(
        run_inner(&parse(dir)).expect("runs"),
        CommandExitCode::new(exit_codes::FAILED)
    );
    let campaign =
        cellgov_fuzz::ppu_reference::run_reference_directory(dir).expect("the directory reads");
    let text = render_ppu_reference(&campaign);
    assert!(text.contains(" 0 failing; "), "{text}");
    assert!(
        text.contains("fuzz ppu-reference: missing facility:dcbz_width\n"),
        "{text}"
    );
}

#[test]
fn a_refused_file_fails_counts_as_failing_and_is_unowned() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_ppu_reference_refused");
    let dir: &Path = &scratch;
    copy_committed(dir, true);
    std::fs::write(dir.join("broken.json"), "{").expect("writes");
    assert_eq!(
        run_inner(&parse(dir)).expect("runs"),
        CommandExitCode::new(exit_codes::FAILED)
    );
    let campaign =
        cellgov_fuzz::ppu_reference::run_reference_directory(dir).expect("the directory reads");
    let text = render_ppu_reference(&campaign);
    assert!(
        text.contains("fuzz ppu-reference: broken.json unit=none refused: "),
        "{text}"
    );
    assert!(text.contains(" 1 failing; "), "{text}");
    assert!(text.contains(" missing=0 "), "{text}");
    assert!(
        text.contains("fuzz ppu-reference: unowned broken.json\n"),
        "{text}"
    );
}

#[test]
fn a_file_given_as_the_directory_is_a_failed_operation() {
    let file = fixtures().join("addi.json");
    let error = run_inner(&parse(&file)).expect_err("a file is not a directory");
    assert!(matches!(error, FuzzCliError::PpuReferenceCampaign(_)));
    assert_eq!(error.exit_code(), exit_codes::FAILED);
}

#[test]
fn an_unreadable_directory_is_a_failed_operation() {
    let missing = fixtures().join("no-such-directory");
    let error = run_inner(&parse(&missing)).expect_err("the directory is missing");
    assert!(matches!(error, FuzzCliError::PpuReferenceCampaign(_)));
    assert_eq!(error.exit_code(), exit_codes::FAILED);
}
