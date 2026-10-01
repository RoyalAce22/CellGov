//! Preflight, start, the wait for the result, and cleanup against an
//! in-memory console: the stale-result rule, the occupied game
//! directory, reclaim, the poll budget, and a cleanup that names
//! everything it could not remove.

use super::*;
use crate::memory_console::{refused, MemoryConsole};

fn target() -> Target {
    Target::new("CGOV00001", "cgov_spu_fixed_value.bin")
}

const CLEAR: &str = "runner_ps3 cleanup --host 10.77.0.2";

#[test]
fn the_target_paths_follow_the_appid_and_the_result_file() {
    assert_eq!(
        target(),
        Target {
            game_dir: "/dev_hdd0/game/CGOV00001".to_string(),
            usrdir: "/dev_hdd0/game/CGOV00001/USRDIR".to_string(),
            result_path: "/dev_hdd0/tmp/cgov_spu_fixed_value.bin".to_string(),
        }
    );
    assert_eq!(target().appid(), "CGOV00001");
}

#[test]
fn a_clean_console_passes_preflight_without_a_change() {
    let mut console = MemoryConsole::empty();
    let mut transcript = Transcript::new();
    preflight(&mut console, &target(), false, CLEAR, &mut transcript).expect("clean");
    assert!(!console
        .calls
        .iter()
        .any(|c| c.starts_with("DELE") || c.starts_with("RMD")));
}

#[test]
fn a_stale_result_is_deleted_once_and_the_run_proceeds() {
    let mut console = MemoryConsole::empty();
    console.put(&target().result_path);
    let mut transcript = Transcript::new();
    preflight(&mut console, &target(), false, CLEAR, &mut transcript).expect("cleared");
    assert!(console.files.is_empty());
    assert_eq!(
        console
            .calls
            .iter()
            .filter(|c| c.starts_with("DELE"))
            .count(),
        1
    );
}

#[test]
fn a_result_that_survives_its_delete_is_refused_with_the_clearing_command() {
    let mut console = MemoryConsole::empty();
    console.put(&target().result_path);
    console.respawn = true;
    let mut transcript = Transcript::new();
    let err =
        preflight(&mut console, &target(), false, CLEAR, &mut transcript).expect_err("refused");
    match err {
        RunnerPs3Error::Refused { reason, clear_with } => {
            assert_eq!(
                reason,
                "a result file remains at /dev_hdd0/tmp/cgov_spu_fixed_value.bin after one delete"
            );
            assert_eq!(clear_with, CLEAR);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        console
            .calls
            .iter()
            .filter(|c| c.starts_with("DELE"))
            .count(),
        1,
        "the rule deletes once"
    );
}

#[test]
fn an_occupied_game_directory_is_refused_unless_reclaimed() {
    let mut console = MemoryConsole::with_package(&target());
    let mut transcript = Transcript::new();
    let err =
        preflight(&mut console, &target(), false, CLEAR, &mut transcript).expect_err("occupied");
    match &err {
        RunnerPs3Error::Refused { reason, clear_with } => {
            assert_eq!(
                reason,
                "/dev_hdd0/game/CGOV00001 already exists on the console"
            );
            assert!(clear_with.ends_with("--reclaim"), "{clear_with}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        console.dirs.contains(&target().game_dir),
        "a refusal changes nothing"
    );

    preflight(&mut console, &target(), true, CLEAR, &mut transcript).expect("reclaimed");
    assert!(!console.dirs.contains(&target().game_dir));
    assert!(console.files.is_empty());
}

#[test]
fn start_mounts_the_eboot_then_starts_it() {
    let mut console = MemoryConsole::with_package(&target());
    let mut transcript = Transcript::new();
    start(&mut console, &target(), &mut transcript).expect("started");
    assert_eq!(
        console.calls,
        [
            "GET /play.ps3/dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN",
            "GET /play.ps3",
        ]
    );
}

#[test]
fn a_start_over_a_missing_eboot_is_a_transport_error() {
    let mut console = MemoryConsole::empty();
    let err = start(&mut console, &target(), &mut Transcript::new()).expect_err("no eboot");
    assert!(
        matches!(
            &err,
            RunnerPs3Error::Transport(TransportError::UnexpectedStatus { status: 404, path })
                if path == "/play.ps3/dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN"
        ),
        "{err:?}"
    );
    assert_eq!(console.calls.len(), 1, "the bare start is never sent");
}

fn started(polls_before_result: usize) -> MemoryConsole {
    let mut console = MemoryConsole::with_package(&target());
    console.on_start = Some((target().result_path, b"frame".to_vec()));
    console.polls_before_result = polls_before_result;
    start(&mut console, &target(), &mut Transcript::new()).expect("started");
    console
}

#[test]
fn the_wait_sleeps_before_each_poll_until_the_result_appears() {
    let mut console = started(2);
    let mut sleeps = Vec::new();
    wait_for_result(
        &mut console,
        &target(),
        30_000,
        500,
        &mut |d| sleeps.push(d),
        &mut Transcript::new(),
    )
    .expect("the third poll finds it");
    assert_eq!(sleeps, [Duration::from_millis(500); 3]);
    assert_eq!(
        fetch_result(&mut console, &target(), 30_000, &mut Transcript::new()).expect("fetched"),
        b"frame"
    );
}

#[test]
fn the_wait_gives_up_after_the_budgets_poll_count_with_the_last_poll_at_the_budget() {
    let mut console = started(usize::MAX);
    let mut sleeps = 0;
    let err = wait_for_result(
        &mut console,
        &target(),
        1_001,
        500,
        &mut |_| sleeps += 1,
        &mut Transcript::new(),
    )
    .expect_err("never appears");
    assert!(
        matches!(
            &err,
            RunnerPs3Error::Timeout {
                timeout_ms: 1_001,
                ..
            }
        ),
        "{err:?}"
    );
    let polls = console
        .calls
        .iter()
        .filter(|c| *c == "GET /dev_hdd0/tmp/cgov_spu_fixed_value.bin")
        .count();
    assert_eq!(
        (polls, sleeps),
        (3, 3),
        "ceil(1001 / 500) polls, the last after 1500 ms of sleep"
    );
}

#[test]
fn a_budget_no_longer_than_one_interval_still_waits_one_interval() {
    let mut console = started(usize::MAX);
    let mut slept = Duration::ZERO;
    wait_for_result(
        &mut console,
        &target(),
        400,
        500,
        &mut |d| slept += d,
        &mut Transcript::new(),
    )
    .expect_err("never appears");
    assert!(
        slept >= Duration::from_millis(400),
        "the only poll comes before the budget ends: {slept:?}"
    );
}

#[test]
fn a_result_left_by_an_earlier_run_is_deleted_before_the_wait_can_accept_it() {
    let mut console = MemoryConsole::with_package(&target());
    console.put(&target().result_path);
    let mut transcript = Transcript::new();
    clear_stale_result(&mut console, &target(), CLEAR, &mut transcript).expect("cleared");
    assert!(!console.files.contains_key(&target().result_path));
    assert!(
        console.dirs.contains(&target().game_dir),
        "the deployed package stays"
    );
}

#[test]
fn a_zero_poll_interval_is_a_usage_error() {
    let err = wait_for_result(
        &mut started(0),
        &target(),
        1_000,
        0,
        &mut |_| {},
        &mut Transcript::new(),
    )
    .expect_err("zero");
    assert_eq!(err.exit_code(), crate::ExitCode::Usage);
}

#[test]
fn cleanup_unmounts_and_removes_the_package_and_the_result() {
    let mut console = MemoryConsole::with_package(&target());
    console.put(&target().result_path);
    let mut transcript = Transcript::new();
    cleanup(&mut console, &target(), &mut transcript).expect("restored");
    assert_eq!(console.unmounts, 1);
    assert!(console.files.is_empty(), "{:?}", console.files);
    assert_eq!(console.dirs.iter().collect::<Vec<_>>(), [GAME_ROOT]);
    assert_eq!(
        transcript.lines().last().map(String::as_str),
        Some("#0001 = console restored")
    );
}

#[test]
fn cleanup_of_an_empty_console_is_a_no_op_that_still_unmounts() {
    let mut console = MemoryConsole::empty();
    let mut transcript = Transcript::new();
    cleanup(&mut console, &target(), &mut transcript).expect("nothing to remove");
    assert_eq!(console.unmounts, 1);
    assert!(
        !console
            .calls
            .iter()
            .any(|c| c.starts_with("DELE") || c.starts_with("RMD")),
        "a game directory its parent does not list is never touched: {:?}",
        console.calls
    );
}

#[test]
fn the_dot_entries_a_listing_carries_are_never_deleted() {
    let mut console = MemoryConsole::with_package(&target());
    let mut transcript = Transcript::new();
    cleanup(&mut console, &target(), &mut transcript).expect("restored");
    let deletes: Vec<&String> = console
        .calls
        .iter()
        .filter(|c| c.starts_with("DELE"))
        .collect();
    assert_eq!(
        deletes,
        [
            "DELE /dev_hdd0/game/CGOV00001/PARAM.SFO",
            "DELE /dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN",
            "DELE /dev_hdd0/game/CGOV00001/USRDIR/spu_main.elf",
        ]
    );
}

#[test]
fn a_cleanup_that_cannot_finish_names_everything_that_remains_and_keeps_going() {
    let mut console = MemoryConsole::with_package(&target());
    console.put(&target().result_path);
    console
        .stubborn
        .insert(format!("{}/EBOOT.BIN", target().usrdir));
    console.unmount_status = Some(500);
    let mut transcript = Transcript::new();
    let err = cleanup(&mut console, &target(), &mut transcript).expect_err("incomplete");
    assert_eq!(err.exit_code(), crate::error::ExitCode::Cleanup);
    match err {
        RunnerPs3Error::Cleanup { remaining } => assert_eq!(
            remaining,
            "the mounted game (unmount failed), \
             /dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN, \
             /dev_hdd0/game/CGOV00001/USRDIR, \
             /dev_hdd0/game/CGOV00001"
        ),
        other => panic!("{other:?}"),
    }
    assert!(
        !console.files.contains_key(&target().result_path),
        "later steps still ran"
    );
    assert!(!console
        .files
        .contains_key(&format!("{}/spu_main.elf", target().usrdir)));
}

#[test]
fn a_result_path_that_answers_neither_200_nor_404_is_a_transport_error() {
    struct Teapot;
    impl ConsoleOps for Teapot {
        fn http_status(&mut self, _: &str, _: &mut Transcript) -> Result<u16, TransportError> {
            Ok(418)
        }
        fn fetch(
            &mut self,
            path: &str,
            _: &mut Transcript,
        ) -> Result<Option<Vec<u8>>, TransportError> {
            Err(refused(path))
        }
        fn list(&mut self, _: &str, _: &mut Transcript) -> Result<Vec<String>, TransportError> {
            Ok(Vec::new())
        }
        fn make_dir(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
            Err(refused(path))
        }
        fn store(
            &mut self,
            path: &str,
            _: &[u8],
            _: &mut Transcript,
        ) -> Result<(), TransportError> {
            Err(refused(path))
        }
        fn delete(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
            Err(refused(path))
        }
        fn remove_dir(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
            Err(refused(path))
        }
    }
    let err =
        preflight(&mut Teapot, &target(), false, CLEAR, &mut Transcript::new()).expect_err("418");
    assert!(
        matches!(
            err,
            RunnerPs3Error::Transport(TransportError::UnexpectedStatus { status: 418, .. })
        ),
        "{err:?}"
    );
}
