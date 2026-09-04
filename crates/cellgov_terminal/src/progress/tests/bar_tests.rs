//! Bar lifecycle: mode capping, teardown, and the registration the
//! panic hook reads.
//!
//! Every bar started here renders in `Plain` or `Off` with no totals
//! emitted, so the render thread writes nothing: an `Ansi` bar would
//! put escape sequences on the harness's stderr from a thread the
//! harness does not capture.

use super::*;
use crate::progress::sink::ProgressSink;
use crate::progress::task::Unit;

/// `LIVE_BAR` is process-global, so the tests that read it run one at
/// a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const QUIET: Task = Task {
    verb: "Verifying",
    tag: "verify",
    phases: &["hashing", "comparing"],
    measured: 0,
    unit: Unit::Bytes,
    items: "files",
    streaming: false,
};

const STREAMING: Task = Task {
    verb: "Booting",
    tag: "boot",
    phases: &["loading", "stepping"],
    measured: 1,
    unit: Unit::Steps,
    items: "",
    streaming: true,
};

fn caps(mode: RenderMode) -> TermCaps {
    TermCaps {
        mode,
        color: mode == RenderMode::Ansi,
        width: 80,
    }
}

#[test]
fn a_streaming_task_caps_the_bar_at_plain() {
    let _s = serial();
    let bar = ProgressBar::start(caps(RenderMode::Ansi), &STREAMING, "wipeout");
    assert_eq!(bar.caps.mode, RenderMode::Plain);
    assert!(!bar.caps.color);
    bar.finish();
}

#[test]
fn an_off_bar_starts_no_thread_and_still_hands_out_a_sink() {
    let bar = ProgressBar::start(caps(RenderMode::Off), &QUIET, "x.pkg");
    assert!(bar.handle.is_none());
    let sink = bar.sink();
    sink.totals(2, 4096);
    sink.advanced(4096);
    sink.item_finished();
    assert_eq!(sink.snapshot().done_amount, 4096);
    bar.finish();
}

#[test]
fn finishing_deregisters_the_bar_the_panic_hook_would_reach() {
    let _s = serial();
    let bar = ProgressBar::start(caps(RenderMode::Plain), &QUIET, "x.pkg");
    assert!(
        live_bar_slot().is_some(),
        "a live render thread must be reachable from the hook"
    );
    bar.finish();
    assert!(live_bar_slot().is_none());
}

#[test]
fn dropping_a_bar_without_a_teardown_still_stops_its_thread() {
    let _s = serial();
    {
        let bar = ProgressBar::start(caps(RenderMode::Plain), &QUIET, "x.pkg");
        assert!(live_bar_slot().is_some());
        drop(bar);
    }
    assert!(
        live_bar_slot().is_none(),
        "Drop must join the render thread, not leak it"
    );
}

#[test]
fn aborting_deregisters_the_bar_too() {
    let _s = serial();
    let bar = ProgressBar::start(caps(RenderMode::Plain), &QUIET, "x.pkg");
    bar.abort();
    assert!(live_bar_slot().is_none());
}

#[test]
fn a_teardown_only_clears_a_registration_that_is_its_own() {
    let _s = serial();
    let mut bar = ProgressBar::start(caps(RenderMode::Off), &QUIET, "x.pkg");
    // An `Off` bar registers nothing, so plant a foreign registration:
    // tearing this bar down must leave the live one reachable.
    let other = Arc::new(BarStop::default());
    *live_bar_slot() = Some(Arc::clone(&other));
    bar.stop_thread();
    let kept = {
        let slot = live_bar_slot();
        slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &other))
    };
    *live_bar_slot() = None;
    assert!(kept, "a bar cleared a registration that was not its own");
}

#[test]
fn only_the_first_teardown_restores_the_terminal() {
    let _s = serial();
    let mut bar = ProgressBar::start(caps(RenderMode::Off), &QUIET, "x.pkg");
    // Stand in for a live `Ansi` bar without spawning a render thread
    // that would write escape sequences past the harness.
    bar.caps.mode = RenderMode::Ansi;
    BAR_ACTIVE.store(true, Ordering::Relaxed);

    let first = bar
        .restore_sequence(true)
        .expect("the live bar owns the restore");
    assert!(first.starts_with("\x1b[?25h"), "the cursor comes back");
    assert!(
        bar.restore_sequence(false).is_none(),
        "a teardown after the panic hook restored must not write again"
    );
    assert!(!BAR_ACTIVE.load(Ordering::Relaxed));
}

/// In release the displaced registration is silent; the assert is the
/// debug-build gate, so the test that names it is too.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "one live bar per process")]
fn starting_a_second_bar_names_the_invariant_it_breaks() {
    let _s = serial();
    let first = ProgressBar::start(caps(RenderMode::Plain), &QUIET, "first.pkg");
    let second = ProgressBar::start(caps(RenderMode::Plain), &QUIET, "second.pkg");
    drop(second);
    drop(first);
}

/// The baseline tick, which writes no line:
///
/// - it crosses no decile,
/// - it is neither the first tick nor the last,
/// - it sits well inside the silence budget.
fn mid_run() -> PlainDue {
    PlainDue {
        total_amount: 100_000_000,
        first: false,
        ending: false,
        decile: 0,
        last_decile: 0,
        since_last: Duration::from_secs(1),
    }
}

#[test]
fn a_plain_task_says_something_at_both_ends_however_little_of_it_runs() {
    assert!(plain_due(&PlainDue {
        first: true,
        ..mid_run()
    }));
    assert!(plain_due(&PlainDue {
        ending: true,
        ..mid_run()
    }));
    assert!(!plain_due(&mid_run()), "a quiet mid-run tick stays quiet");
}

#[test]
fn a_plain_task_without_a_denominator_answers_the_silence_budget_and_nothing_else() {
    assert!(plain_due(&PlainDue {
        total_amount: 0,
        since_last: PLAIN_SILENCE,
        ..mid_run()
    }));
    for quiet in [
        PlainDue {
            total_amount: 0,
            first: true,
            ..mid_run()
        },
        PlainDue {
            total_amount: 0,
            ending: true,
            ..mid_run()
        },
        PlainDue {
            total_amount: 0,
            decile: 5,
            ..mid_run()
        },
    ] {
        assert!(!plain_due(&quiet));
    }
}

#[test]
fn a_plain_task_that_finishes_inside_its_first_tick_still_speaks() {
    assert!(plain_due(&PlainDue {
        first: true,
        ending: true,
        ..mid_run()
    }));
}

#[test]
fn a_plain_task_stays_quiet_for_a_ratio_that_went_backwards() {
    assert!(!plain_due(&PlainDue {
        decile: 3,
        last_decile: 7,
        ..mid_run()
    }));
}

#[test]
fn a_plain_task_stays_quiet_one_tick_short_of_the_silence_budget() {
    assert!(!plain_due(&PlainDue {
        since_last: PLAIN_SILENCE - TICK,
        ..mid_run()
    }));
}

#[test]
fn a_plain_task_speaks_on_a_decile_or_after_the_silence_budget() {
    assert!(plain_due(&PlainDue {
        decile: 1,
        ..mid_run()
    }));
    assert!(
        !plain_due(&PlainDue {
            decile: 4,
            last_decile: 4,
            ..mid_run()
        }),
        "a decile already reported must not repeat"
    );
    assert!(plain_due(&PlainDue {
        since_last: PLAIN_SILENCE,
        ..mid_run()
    }));
}

/// `BAR_ACTIVE` is false for every bar these tests start, so the
/// release writes no escape sequence past the harness.
#[test]
fn releasing_the_terminal_with_no_live_bar_is_a_no_op_however_often_it_runs() {
    let _s = serial();
    release_terminal();
    release_terminal();
    assert!(live_bar_slot().is_none());
    assert!(!BAR_ACTIVE.load(Ordering::Relaxed));
}

#[test]
fn releasing_the_terminal_stops_the_bar_and_leaves_its_teardown_safe() {
    let _s = serial();
    let bar = ProgressBar::start(caps(RenderMode::Plain), &QUIET, "x.pkg");
    assert!(live_bar_slot().is_some());
    release_terminal();
    assert!(
        live_bar_slot().is_none(),
        "the release must deregister the bar the panic hook would reach"
    );
    let other = Arc::new(BarStop::default());
    *live_bar_slot() = Some(Arc::clone(&other));
    bar.finish();
    let kept = {
        let slot = live_bar_slot();
        slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &other))
    };
    *live_bar_slot() = None;
    assert!(
        kept,
        "a released bar cleared a registration it no longer owns"
    );
}

#[test]
fn the_stop_handshake_reports_an_exit_and_times_out_without_one() {
    let stop = Arc::new(BarStop::default());

    // Nobody is running, so the wait spends its whole budget and
    // returns rather than blocking the panic hook forever.
    let t0 = Instant::now();
    stop.wait_for_exit(Duration::from_millis(30));
    assert!(t0.elapsed() >= Duration::from_millis(30));

    // A thread that leaves its loop releases the wait early.
    let s = Arc::clone(&stop);
    let h = std::thread::spawn(move || {
        while !s.wait_tick() {}
        s.mark_exited();
    });
    stop.request_stop();
    stop.wait_for_exit(Duration::from_secs(5));
    assert!(
        stop.lock().exited,
        "the wait returned before the thread marked its exit"
    );
    h.join().expect("render stand-in joins");
}
