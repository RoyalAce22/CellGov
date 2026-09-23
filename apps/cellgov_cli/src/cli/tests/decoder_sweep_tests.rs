use super::*;

#[test]
fn command_line_requires_an_explicit_scan_scope() {
    use crate::cli::parse::{try_parse, Command, DevCommand};
    let args = [
        "cellgov",
        "dev",
        "decoder-sweep",
        "spu",
        "--full",
        "--shard",
        "1",
        "--shards",
        "4",
        "--output",
        "result.json",
    ];
    let cli = try_parse(&args.map(str::to_string)).expect("full shard must parse");
    let Command::Dev(DevCommand::DecoderSweep(parsed)) = cli.command else {
        panic!("decoder sweep must remain a dev command");
    };
    assert!(matches!(parsed.decoder, SweepDecoder::Spu));
    assert!(parsed.full);
    assert_eq!((parsed.shard, parsed.shards), (Some(1), Some(4)));
    let conflict = try_parse(
        &[
            "cellgov",
            "dev",
            "decoder-sweep",
            "ppu",
            "--full",
            "--count",
            "3",
            "--output",
            "out.json",
        ]
        .map(str::to_string),
    )
    .expect_err("full and bounded scopes conflict");
    assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);
    for flags in [
        vec!["--count", "1", "--shard", "0"],
        vec!["--count", "1", "--shards", "1"],
        vec!["--full", "--start", "0"],
    ] {
        let mut argv = vec!["cellgov", "dev", "decoder-sweep", "ppu"];
        argv.extend(flags);
        argv.extend(["--output", "out.json"]);
        let error = try_parse(&argv.iter().map(ToString::to_string).collect::<Vec<_>>())
            .expect_err("inapplicable flag must refuse");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "{argv:?}"
        );
    }
    let missing = try_parse(
        &[
            "cellgov",
            "dev",
            "decoder-sweep",
            "ppu",
            "--output",
            "out.json",
        ]
        .map(str::to_string),
    )
    .expect_err("one scan scope is required");
    assert_eq!(
        missing.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[test]
fn bounded_cli_scan_writes_a_replayable_ppu_artifact() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("decoder_sweep_bounded");
    let output = scratch.join("ppu.json");
    let args = DecoderSweepArgs {
        decoder: SweepDecoder::Ppu,
        full: false,
        start: None,
        count: Some(257),
        shard: None,
        shards: None,
        chunk_size: 17,
        workers: 3,
        cancel_after: None,
        output: output.clone(),
    };
    let exit = run_inner(&args).expect("bounded scan must run");
    assert_eq!(exit, CommandExitCode::SUCCESS);
    let artifact: cellgov_fuzz::raw_decode::RawDecodeArtifact =
        serde_json::from_slice(&std::fs::read(&output).expect("output must exist"))
            .expect("versioned output must parse");
    assert_eq!(artifact.decoder, RawDecoder::Ppu);
    assert_eq!(artifact.processed, 257);
    assert_eq!(artifact.accepted + artifact.refused + artifact.panics, 257);
    assert_eq!(artifact.word_at(256), Some(256));
}

#[test]
fn cancellation_writes_an_explicit_partial_artifact_and_fails_the_gate() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("decoder_sweep_cancelled");
    let output = scratch.join("spu.json");
    let args = DecoderSweepArgs {
        decoder: SweepDecoder::Spu,
        full: false,
        start: None,
        count: Some(10),
        shard: None,
        shards: None,
        chunk_size: 4,
        workers: 2,
        cancel_after: Some(3),
        output: output.clone(),
    };
    let exit = run_inner(&args).expect("cancelled scan must produce evidence");
    assert_eq!(exit.value(), super::super::exit_codes::FAILED as u8);
    let artifact: cellgov_fuzz::raw_decode::RawDecodeArtifact =
        serde_json::from_slice(&std::fs::read(&output).expect("partial artifact must exist"))
            .expect("partial artifact must parse");
    assert_eq!(
        artifact.status,
        cellgov_fuzz::raw_decode::RawDecodeStatus::Cancelled
    );
    assert_eq!(artifact.processed, 3);
    assert!(!artifact.is_clean());
}

#[test]
fn invalid_scope_and_output_refusals_preserve_typed_causes() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("decoder_sweep_refusals");
    let base = DecoderSweepArgs {
        decoder: SweepDecoder::Ppu,
        full: false,
        start: None,
        count: None,
        shard: None,
        shards: None,
        chunk_size: 1,
        workers: 1,
        cancel_after: None,
        output: scratch.join("missing.json"),
    };
    assert!(matches!(
        run_inner(&base),
        Err(DecoderSweepError::Selection)
    ));
    let missing = run(&base).expect_err("missing scope must use the usage status");
    assert_eq!(
        missing.code().expect("usage has an exit code").value(),
        super::super::exit_codes::USAGE as u8
    );
    let mut invalid = DecoderSweepArgs {
        count: Some(1),
        ..base
    };
    invalid.output = scratch.join("missing-parent").join("nested.json");
    assert!(matches!(
        run_inner(&invalid),
        Err(DecoderSweepError::Write { .. })
    ));
    let bad_chunk = DecoderSweepArgs {
        chunk_size: 0,
        output: scratch.join("bad-chunk.json"),
        ..invalid
    };
    let invalid = run(&bad_chunk).expect_err("zero chunk must be a usage error");
    assert_eq!(
        invalid.code().expect("usage has an exit code").value(),
        super::super::exit_codes::USAGE as u8
    );
}

#[test]
fn closed_stdout_preserves_the_broken_pipe_status() {
    struct RefuseWrite(std::io::ErrorKind);
    impl std::io::Write for RefuseWrite {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(self.0))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let closed = write_report(&mut RefuseWrite(std::io::ErrorKind::BrokenPipe), "result\n")
        .expect("closed pipe is a typed exit");
    assert_eq!(closed.value(), super::super::exit_codes::BROKEN_PIPE as u8);
    assert!(matches!(
        write_report(
            &mut RefuseWrite(std::io::ErrorKind::PermissionDenied),
            "result\n"
        ),
        Err(DecoderSweepError::Stdout(_))
    ));
}
