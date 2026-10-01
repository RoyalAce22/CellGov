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

/// A reclaim question answered yes.
fn yes(_: &Reclaim) -> Result<bool, RunnerPs3Error> {
    Ok(true)
}

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
    preflight(
        &mut console,
        &target(),
        false,
        &mut yes,
        CLEAR,
        &mut transcript,
    )
    .expect("clean");
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
    preflight(
        &mut console,
        &target(),
        false,
        &mut yes,
        CLEAR,
        &mut transcript,
    )
    .expect("cleared");
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
    let err = preflight(
        &mut console,
        &target(),
        false,
        &mut yes,
        CLEAR,
        &mut transcript,
    )
    .expect_err("refused");
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
    let err = preflight(
        &mut console,
        &target(),
        false,
        &mut yes,
        CLEAR,
        &mut transcript,
    )
    .expect_err("occupied");
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

    let mut asked = Vec::new();
    preflight(
        &mut console,
        &target(),
        true,
        &mut |question| {
            asked.push(question.clone());
            Ok(true)
        },
        CLEAR,
        &mut transcript,
    )
    .expect("reclaimed");
    assert_eq!(
        asked,
        [Reclaim {
            game_dir: "/dev_hdd0/game/CGOV00001".to_string(),
            contents: vec![
                "/dev_hdd0/game/CGOV00001/PARAM.SFO".to_string(),
                "/dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN".to_string(),
                "/dev_hdd0/game/CGOV00001/USRDIR/spu_main.elf".to_string(),
                "/dev_hdd0/game/CGOV00001/USRDIR".to_string(),
            ],
        }],
        "the question names the directory and everything in it"
    );
    assert!(!console.dirs.contains(&target().game_dir));
    assert!(console.files.is_empty());
}

#[test]
fn a_declined_or_unaskable_reclaim_leaves_the_console_as_it_was() {
    for (answer, refused) in [
        (Ok(false), true),
        (
            Err(RunnerPs3Error::Usage("this run may not prompt".to_string())),
            false,
        ),
    ] {
        let mut console = MemoryConsole::with_package(&target());
        let before = (console.files.clone(), console.dirs.clone());
        let mut answer = Some(answer);
        let err = preflight(
            &mut console,
            &target(),
            true,
            &mut |_| answer.take().expect("asked once"),
            CLEAR,
            &mut Transcript::new(),
        )
        .expect_err("not reclaimed");
        match err {
            RunnerPs3Error::Refused { reason, clear_with } if refused => {
                assert_eq!(
                    reason,
                    "/dev_hdd0/game/CGOV00001 already exists on the console, and the reclaim \
                     was declined"
                );
                assert_eq!(clear_with, CLEAR);
            }
            RunnerPs3Error::Usage(message) if !refused => {
                assert_eq!(message, "this run may not prompt");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!((console.files, console.dirs), before, "nothing removed");
        assert!(
            !console
                .calls
                .iter()
                .any(|c| c.starts_with("DELE") || c.starts_with("RMD") || c.contains("unmount")),
            "{:?}",
            console.calls
        );
    }
}

#[test]
fn start_sends_one_request_naming_the_appid() {
    let mut console = MemoryConsole::with_package(&target());
    let mut transcript = Transcript::new();
    start(&mut console, &target(), &mut transcript).expect("started");
    assert_eq!(console.calls, ["GET /play.ps3?CGOV00001"]);
}

/// A status page whose `MEM:` line reads `mem`, marked up as webMAN
/// serves it.
fn status_page(mem: &str) -> Vec<u8> {
    format!(
        "<b>CPU: 67&deg;C</b><br><a class=\"s\" href=\"/browser.ps3$slaunch\">MEM: {mem}</a>\
         <br>HDD:  97.5 GB free<br>"
    )
    .into_bytes()
}

#[test]
fn the_status_page_says_whether_the_console_is_at_the_xmb() {
    let page = |mem: &str| String::from_utf8(status_page(mem)).expect("ascii");
    assert_eq!(crate::console::at_xmb(&page("1,748 KB (XMB)")), Some(true));
    assert_eq!(crate::console::at_xmb(&page("1,316 KB ")), Some(false));
    assert_eq!(crate::console::at_xmb("<b>CPU: 67&deg;C</b>"), None);
}

#[test]
fn a_launch_is_seen_when_the_console_leaves_the_xmb() {
    let mut console = MemoryConsole::with_package(&target());
    console.status_refusals = 1;
    console
        .files
        .insert(STATUS_PATH.to_string(), status_page("1,748 KB (XMB)"));
    console.later_status.push_back(status_page("1,316 KB "));
    let mut transcript = Transcript::new();
    wait_for_launch(
        &mut console,
        &target(),
        30_000,
        500,
        &mut |_| {},
        &mut transcript,
    )
    .expect("left on the third poll");
    assert!(
        transcript
            .lines()
            .iter()
            .any(|l| l.ends_with("left the XMB after 3 poll(s)")),
        "{:?}",
        transcript.lines()
    );
}

#[test]
fn a_test_that_ran_between_two_polls_counts_as_launched() {
    let mut console = MemoryConsole::with_package(&target());
    console
        .files
        .insert(STATUS_PATH.to_string(), status_page("1,748 KB (XMB)"));
    console.put(&target().result_path);
    let mut transcript = Transcript::new();
    wait_for_launch(
        &mut console,
        &target(),
        30_000,
        500,
        &mut |_| {},
        &mut transcript,
    )
    .expect("the result shows it ran");
    assert!(
        transcript
            .lines()
            .iter()
            .any(|l| l.ends_with("the result was already there at poll 1")),
        "{:?}",
        transcript.lines()
    );
}

#[test]
fn an_ignored_start_is_sent_once_more_after_ten_seconds() {
    let mut console = MemoryConsole::with_package(&target());
    console.ignored_starts = 1;
    console.on_start = Some((target().result_path, b"frame".to_vec()));
    console
        .files
        .insert(STATUS_PATH.to_string(), status_page("1,748 KB (XMB)"));
    start(&mut console, &target(), &mut Transcript::new()).expect("answered");
    let mut transcript = Transcript::new();
    wait_for_launch(
        &mut console,
        &target(),
        30_000,
        500,
        &mut |_| {},
        &mut transcript,
    )
    .expect("the second start took");
    let starts = console
        .calls
        .iter()
        .filter(|c| *c == "GET /play.ps3?CGOV00001")
        .count();
    assert_eq!(starts, 2, "{:?}", console.calls);
    assert!(
        transcript
            .lines()
            .iter()
            .any(|l| l.ends_with("still at the XMB after 20 poll(s); sending the start once more")),
        "{:?}",
        transcript.lines()
    );
}

#[test]
fn a_result_still_being_written_counts_as_not_yet() {
    let mut console = started(0);
    console.result_path = Some(target().result_path);
    console.growing_reads = 2;
    let mut transcript = Transcript::new();
    wait_for_result(
        &mut console,
        &target(),
        30_000,
        500,
        &mut |_| {},
        &mut transcript,
    )
    .expect("whole on the third poll");
    assert!(
        transcript
            .lines()
            .iter()
            .any(|l| l.ends_with("result present after 3 poll(s)")),
        "{:?}",
        transcript.lines()
    );
    assert_eq!(
        transcript
            .lines()
            .iter()
            .filter(|l| l.ends_with("grew from 4 to 16 bytes during the read; not yet"))
            .count(),
        2
    );
}

#[test]
fn a_start_that_never_leaves_the_xmb_is_not_started() {
    let mut console = MemoryConsole::with_package(&target());
    console
        .files
        .insert(STATUS_PATH.to_string(), status_page("1,748 KB (XMB)"));
    let mut polls = 0;
    let err = wait_for_launch(
        &mut console,
        &target(),
        1_001,
        500,
        &mut |_| polls += 1,
        &mut Transcript::new(),
    )
    .expect_err("never left");
    assert!(
        matches!(err, RunnerPs3Error::NotStarted { timeout_ms: 1_001 }),
        "{err:?}"
    );
    assert_eq!(err.exit_code(), crate::ExitCode::Timeout);
    assert_eq!(polls, 3);
}

#[test]
fn the_xmb_wait_counts_refused_and_in_game_polls_as_not_yet() {
    let mut console = MemoryConsole::empty();
    console.status_refusals = 2;
    console
        .files
        .insert(STATUS_PATH.to_string(), status_page("1,316 KB "));
    console.later_status.push_back(status_page("1,316 KB "));
    console
        .later_status
        .push_back(status_page("1,748 KB (XMB)"));
    let mut sleeps = Vec::new();
    let mut transcript = Transcript::new();
    wait_for_xmb(
        &mut console,
        30_000,
        500,
        &mut |d| sleeps.push(d),
        &mut transcript,
    )
    .expect("the fifth poll finds the XMB");
    assert_eq!(sleeps, [Duration::from_millis(500); 5]);
    assert!(
        transcript
            .lines()
            .iter()
            .any(|l| l.ends_with("back at the XMB after 5 poll(s)")),
        "{:?}",
        transcript.lines()
    );
}

#[test]
fn a_title_that_never_returns_to_the_xmb_is_still_running_after_the_budget() {
    for page in [status_page("1,316 KB "), b"<b>no memory line</b>".to_vec()] {
        let mut console = MemoryConsole::empty();
        console.files.insert(STATUS_PATH.to_string(), page);
        let mut polls = 0;
        let err = wait_for_xmb(
            &mut console,
            1_001,
            500,
            &mut |_| polls += 1,
            &mut Transcript::new(),
        )
        .expect_err("never back");
        assert!(
            matches!(err, RunnerPs3Error::StillRunning { timeout_ms: 1_001 }),
            "{err:?}"
        );
        assert_eq!(err.exit_code(), crate::ExitCode::Timeout);
        assert_eq!(polls, 3, "ceil(1001 / 500) polls");
    }
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
    assert_eq!(
        console.dirs.iter().collect::<Vec<_>>(),
        [GAME_ROOT, RESULT_ROOT]
    );
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
    let err = preflight(
        &mut Teapot,
        &target(),
        false,
        &mut yes,
        CLEAR,
        &mut Transcript::new(),
    )
    .expect_err("418");
    assert!(
        matches!(
            err,
            RunnerPs3Error::Transport(TransportError::UnexpectedStatus { status: 418, .. })
        ),
        "{err:?}"
    );
}

#[test]
fn a_console_that_does_not_answer_is_named_by_its_address() {
    // A port that held a listener a moment ago and holds none now.
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .expect("a free port")
        .port();
    let mut console = WebmanConsole::new(Endpoint {
        http_port: port,
        io_timeout_ms: 2_000,
        ..Endpoint::new("127.0.0.1")
    });
    let error = console
        .fetch("/cpursx.ps3", &mut Transcript::new())
        .expect_err("nothing listens");
    assert!(
        matches!(&error, TransportError::Unreachable { address, .. } if *address == format!("127.0.0.1:{port}")),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .starts_with(&format!("connect to 127.0.0.1:{port}: ")),
        "{error}"
    );
}
