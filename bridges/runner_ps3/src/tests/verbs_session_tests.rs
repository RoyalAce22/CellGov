//! A verb ends the console's session as its last exchange.

use super::*;

/// Whether the console's last session end follows its last request.
fn ended_last(console: &MemoryConsole) -> bool {
    console.session_ends.last() == Some(&console.calls.len())
}

#[test]
fn a_verb_that_changes_the_console_ends_its_session_after_the_lease() {
    let mut bench = Bench::new();
    for verb in ["deploy", "cleanup"] {
        bench.on_console(verb, &[]).expect(verb);
        assert_eq!(
            bench.console.calls.last().map(String::as_str),
            Some(format!("RMD {LEASE_DIR}").as_str()),
            "{verb}"
        );
        assert!(
            ended_last(&bench.console),
            "{verb}: {:?} {:?}",
            bench.console.session_ends,
            bench.console.calls
        );
    }
}

#[test]
fn a_capture_ends_its_session_before_the_start_and_after_the_lease() {
    let mut bench = Bench::new();
    bench.arm();
    bench
        .on_console("capture", &["--harness-revision", "0123456789abcdef"])
        .expect("capture");
    let play = bench
        .console
        .calls
        .iter()
        .position(|call| call.starts_with("GET /play.ps3?"))
        .expect("started");
    assert!(
        bench.console.session_ends.contains(&play),
        "{:?} {:?}",
        bench.console.session_ends,
        bench.console.calls
    );
    assert!(ended_last(&bench.console));
}

#[test]
fn a_refused_lease_ends_the_session() {
    let mut bench = Bench::new();
    let held = bench.lease("another@elsewhere");
    bench.on_console("cleanup", &[]).expect_err("held");
    assert!(
        ended_last(&bench.console),
        "{:?}",
        bench.console.session_ends
    );
    held.release(&mut bench.console, &mut Transcript::new())
        .expect("release");
}

#[test]
fn unlock_ends_its_session() {
    let mut bench = Bench::new();
    drop(bench.lease("stale@elsewhere"));
    bench.execute(&["unlock"]).expect("unlock");
    assert!(
        ended_last(&bench.console),
        "{:?}",
        bench.console.session_ends
    );
}
