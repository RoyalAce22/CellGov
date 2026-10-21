//! Where the run steps end the console's session.

use super::*;
use crate::memory_console::MemoryConsole;

fn target() -> Target {
    Target::new("CGOV00001", "cgov_spu_fixed_value.bin")
}

#[test]
fn the_session_ends_just_before_the_start_request() {
    let mut console = MemoryConsole::with_package(&target());
    let mut transcript = Transcript::new();
    console
        .make_dir("/dev_hdd0/tmp/before", &mut transcript)
        .expect("MKD");
    start(&mut console, &target(), &mut transcript).expect("started");
    assert_eq!(
        console.calls,
        ["MKD /dev_hdd0/tmp/before", "GET /play.ps3?CGOV00001"]
    );
    assert_eq!(console.session_ends, [1]);
}

#[test]
fn the_session_ends_before_the_reclaim_question() {
    let mut console = MemoryConsole::with_package(&target());
    let error = preflight(
        &mut console,
        &target(),
        true,
        &mut |_| Ok(false),
        "runner_ps3 cleanup --host 10.77.0.2",
        &mut Transcript::new(),
    )
    .expect_err("declined");
    assert!(matches!(error, RunnerPs3Error::Refused { .. }), "{error:?}");
    // A declined question returns at once, so the console records only
    // an end sent before the question.
    assert_eq!(console.session_ends, [console.calls.len()]);
}
