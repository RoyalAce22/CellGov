use std::path::{Path, PathBuf};

use super::entry::run_inner;
use super::error::FuzzCliError;
use super::reference::render_spu_reference;

use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::{try_parse, Command, DevCommand, FuzzArgs, FuzzCommand};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/cellgov_fuzz/tests/fixtures/spu_reference")
}

fn parse(dir: &Path) -> FuzzArgs {
    let argv = [
        "cellgov",
        "dev",
        "fuzz",
        "spu-reference",
        dir.to_str().expect("utf-8 path"),
    ];
    let cli = try_parse(&argv.iter().map(ToString::to_string).collect::<Vec<_>>())
        .expect("spu-reference parses");
    let Command::Dev(DevCommand::Fuzz(fuzz)) = cli.command else {
        panic!("fuzz must remain a dev command");
    };
    assert!(matches!(fuzz.command, FuzzCommand::SpuReference(_)));
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
        cellgov_fuzz::spu_reference::run_reference_directory(&dir).expect("the directory reads");
    let text = render_spu_reference(&campaign);
    assert!(text.contains(
        "fuzz spu-reference: dfa.json directed-rounding-per-slice unit=instruction:dfa match \
         differences=[] unchosen=[] excluded=2"
    ));
    assert!(text.contains(
        "fuzz spu-reference: rotqbyi.json sequential-bytes-by-12 unit=instruction:rotqbyi \
         match differences=[] unchosen=[] excluded=2"
    ));
    assert!(text.contains("missing=0 duplicated=0 unowned=0 stale_pending=0 unknown_pending=0"));
}

#[test]
fn a_differing_file_or_a_gap_fails_and_is_named() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_spu_reference");
    let dir: &Path = &scratch;
    let fixture =
        std::fs::read_to_string(fixtures().join("../spu_reference_single/rotqbyi_12_v1.json"))
            .expect("reads");
    let mut wrong: serde_json::Value = serde_json::from_str(&fixture).expect("JSON");
    wrong["expected"]["pc"]["value"] = 8.into();
    std::fs::write(dir.join("rotqbyi.json"), wrong.to_string()).expect("writes");
    let failed = run_inner(&parse(dir)).expect("runs");
    assert_eq!(failed, CommandExitCode::new(exit_codes::FAILED));
    let campaign =
        cellgov_fuzz::spu_reference::run_reference_directory(dir).expect("the directory reads");
    let text = render_spu_reference(&campaign);
    assert!(text.contains("rotqbyi.json single unit=instruction:rotqbyi differs"));
    assert!(text.contains("fuzz spu-reference: missing facility:events\n"));
}

#[test]
fn an_unreadable_directory_is_a_failed_operation() {
    let missing = fixtures().join("no-such-directory");
    let error = run_inner(&parse(&missing)).expect_err("the directory is missing");
    assert!(matches!(error, FuzzCliError::SpuReferenceCampaign(_)));
    assert_eq!(error.exit_code(), exit_codes::FAILED);
}
