//! What the sink accumulates, read back through a snapshot.

use super::*;

#[test]
fn the_sink_accumulates_what_a_frame_needs() {
    let s = ProgressState::new();
    s.phase(2);
    s.totals(10, 4096);
    s.item_started("PS3_GAME/USRDIR/EBOOT.BIN");
    s.advanced(1024);
    s.advanced(1024);
    s.item_finished();

    let snap = s.snapshot();
    assert_eq!(snap.phase, 2);
    assert_eq!((snap.total_items, snap.done_items), (10, 1));
    assert_eq!((snap.total_amount, snap.done_amount), (4096, 2048));
    assert_eq!(snap.current, "PS3_GAME/USRDIR/EBOOT.BIN");
    assert!(!s.finished.load(std::sync::atomic::Ordering::Relaxed));
    s.finished();
    assert!(s.finished.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn a_started_item_replaces_the_previous_one() {
    let s = ProgressState::new();
    s.item_started("first-with-a-long-name");
    s.item_started("second");
    assert_eq!(s.snapshot().current, "second");
}

#[test]
fn a_preset_counts_toward_the_ratio_but_not_the_rate() {
    let s = ProgressState::new();
    s.totals(1, 1000);
    s.preset_done(400);
    let snap = s.snapshot();
    assert_eq!(snap.done_amount, 400, "the bar opens at the preset");
    assert_eq!(snap.advanced(), 0, "nothing was transferred this run yet");

    s.advanced(100);
    let snap = s.snapshot();
    assert_eq!(snap.done_amount, 500);
    assert_eq!(snap.advanced(), 100);
}

#[test]
fn a_preset_never_walks_the_done_amount_backwards() {
    let s = ProgressState::new();
    s.advanced(700);
    s.preset_done(400);
    assert_eq!(s.snapshot().done_amount, 700);
    assert_eq!(
        s.snapshot().advanced(),
        300,
        "saturating: the preset is not re-attributed to this run"
    );
}

#[test]
fn a_repeated_preset_never_manufactures_throughput() {
    let s = ProgressState::new();
    s.preset_done(400);
    s.preset_done(200);
    let snap = s.snapshot();
    assert_eq!(snap.done_amount, 400);
    assert_eq!(snap.preset_amount, 400);
    assert_eq!(snap.advanced(), 0);
}

#[test]
fn the_no_op_sink_swallows_every_event() {
    let sink: &dyn ProgressSink = &();
    sink.phase(3);
    sink.totals(1, 2);
    sink.preset_done(4);
    sink.item_started("x");
    sink.advanced(8);
    sink.item_finished();
    sink.finished();
}
