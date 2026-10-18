use std::path::Path;

use cellgov_fuzz::spu_documented::{
    DocumentedDifference, DocumentedReplay, Question, StoppedVector,
};

use super::documented::render_spu_documented;
use super::entry::run_inner;

use crate::cli::exit::CommandExitCode;
use crate::cli::parse::{try_parse, Command, DevCommand, FuzzArgs, FuzzCommand};

fn parse(output: &Path) -> FuzzArgs {
    let argv = [
        "cellgov",
        "dev",
        "fuzz",
        "spu-documented",
        "--output",
        output.to_str().expect("utf-8 path"),
    ];
    let cli = try_parse(&argv.iter().map(ToString::to_string).collect::<Vec<_>>())
        .expect("spu-documented parses");
    let Command::Dev(DevCommand::Fuzz(fuzz)) = cli.command else {
        panic!("fuzz must remain a dev command");
    };
    assert!(matches!(fuzz.command, FuzzCommand::SpuDocumented(_)));
    fuzz
}

#[test]
fn the_list_replays_clean_and_is_written_one_vector_per_line() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_spu_documented");
    let output = scratch.join("spu_documented.jsonl");
    assert_eq!(
        run_inner(&parse(&output)).expect("runs"),
        CommandExitCode::SUCCESS
    );
    let text = std::fs::read_to_string(&output).expect("the list was written");
    let vectors = cellgov_fuzz::spu_documented::documented_vectors();
    assert_eq!(text.lines().count(), vectors.len());
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["id"], vectors[0].id.as_str());
}

#[test]
fn a_difference_is_named_and_the_undefined_counts_are_summed() {
    let replay = DocumentedReplay {
        vectors: 2,
        documented: 10,
        undefined: [
            (Question::EstimateTables, 3),
            (Question::ConversionScale, 4),
        ]
        .into(),
        differences: vec![DocumentedDifference {
            vector: "fa/cell/0000".to_owned(),
            component: "r3.w0".to_owned(),
            mask: 0xFFFF_FFFF << 96,
            expected: 1 << 96,
            observed: 0,
        }],
        stopped: vec![StoppedVector {
            vector: "cflts/undefined-i8/000".to_owned(),
            step: 0,
            reason: "Fault(UndefinedConversionScale(0))".to_owned(),
        }],
    };
    let text = render_spu_documented(&replay);
    assert!(
        text.contains("fuzz spu-documented: fa/cell/0000 r3.w0 differs"),
        "{text}"
    );
    assert!(
        text.contains("fuzz spu-documented: cflts/undefined-i8/000 stopped at instruction 0: "),
        "{text}"
    );
    assert!(
        text.ends_with(
            "2 vector(s), 1 stopped, 10 documented component(s), 1 differing; undefined 7 \
             [estimate-tables=3 conversion-scale=4]\n"
        ),
        "{text}"
    );
}
