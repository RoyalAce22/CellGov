//! The measured phase with no denominator: it shows a tally and
//! predicts nothing.

use super::tests::visible_lines;
use super::*;
use crate::progress::task::Unit;

const BOOT: Task = Task {
    verb: "Booting",
    tag: "boot",
    phases: &["loading", "stepping"],
    measured: 1,
    unit: Unit::Steps,
    items: "",
    streaming: true,
};

const LOADING: u8 = 0;
const STEPPING: u8 = 1;

fn stepping(total_amount: u64, done_amount: u64) -> Snapshot {
    Snapshot {
        phase: STEPPING,
        total_amount,
        done_amount,
        preset_amount: 0,
        total_items: 0,
        done_items: 0,
        current: String::new(),
    }
}

fn ctx(eta: Option<u64>) -> FrameCtx<'static> {
    FrameCtx {
        task: &BOOT,
        label: "synthetic",
        width: 80,
        color: false,
        first: true,
        ratio: 0.0,
        rate: 1_200_000.0,
        eta,
        elapsed_secs: 12,
        spinner: '|',
        done: false,
    }
}

#[test]
fn a_counting_phase_renders_its_tally_and_elapsed_time_with_no_eta() {
    let f = compose_frame(&stepping(0, 43_040), &ctx(None));
    let lines = visible_lines(&f);
    assert_eq!(lines[0], "Booting synthetic  [stepping]");
    assert_eq!(lines[1], "| stepping...", "no bar without a finish line");
    assert_eq!(lines[2], "43.0k steps  1.2M steps/s  elapsed 12s");
    assert!(!f.contains('%') && !f.contains("ETA"), "{f}");
}

#[test]
fn a_phase_before_the_measured_one_counts_nothing() {
    let f = compose_frame(
        &Snapshot {
            phase: LOADING,
            ..stepping(0, 0)
        },
        &ctx(None),
    );
    let lines = visible_lines(&f);
    assert_eq!(lines[1], "| loading...");
    assert_eq!(lines[2], "", "{f}");
}

#[test]
fn a_counting_phase_with_no_rate_yet_still_reports_its_tally_and_elapsed_time() {
    let f = compose_frame(
        &stepping(0, 8_192),
        &FrameCtx {
            rate: 0.0,
            elapsed_secs: 0,
            ..ctx(None)
        },
    );
    assert_eq!(visible_lines(&f)[2], "8.2k steps  elapsed 0s");
}

#[test]
fn a_measured_phase_with_a_finish_line_shows_its_eta_and_no_elapsed_time() {
    let f = compose_frame(
        &stepping(40_000, 10_000),
        &FrameCtx {
            ratio: 0.25,
            ..ctx(Some(25))
        },
    );
    let lines = visible_lines(&f);
    assert!(lines[1].contains(" 25%  10.0k / 40.0k"), "{}", lines[1]);
    assert_eq!(lines[2], "1.2M steps/s  ETA 25s");
}

#[test]
fn a_run_past_its_finish_line_shows_the_true_count_at_one_hundred_percent() {
    let f = compose_frame(
        &stepping(40_000, 44_100),
        &FrameCtx {
            ratio: 1.0,
            ..ctx(None)
        },
    );
    let lines = visible_lines(&f);
    assert!(lines[1].contains(" 100%  44.1k / 40.0k"), "{}", lines[1]);
    assert_eq!(
        lines[2], "1.2M steps/s",
        "a phase with a finish line is not a counting line: no elapsed time"
    );
}

#[test]
fn the_final_frame_of_a_counting_phase_keeps_its_tally() {
    let f = compose_frame(
        &stepping(0, 43_040),
        &FrameCtx {
            ratio: 1.0,
            spinner: '=',
            done: true,
            ..ctx(None)
        },
    );
    let lines = visible_lines(&f);
    assert!(lines[0].ends_with("[done]"), "{}", lines[0]);
    assert_eq!(lines[1], "= done");
    assert_eq!(lines[2], "43.0k steps  1.2M steps/s  elapsed 12s");
}

#[test]
fn a_counting_frame_fits_the_forty_column_floor() {
    for (done, elapsed) in [(0u64, 0u64), (999_999_999, 359_999), (u64::MAX, u64::MAX)] {
        let f = compose_frame(
            &stepping(0, done),
            &FrameCtx {
                width: 40,
                rate: 999_999_999.0,
                elapsed_secs: elapsed,
                ..ctx(None)
            },
        );
        let lines = visible_lines(&f);
        assert_eq!(lines.len(), 3, "done {done} elapsed {elapsed}: {f:?}");
        for (n, line) in lines.iter().enumerate() {
            assert!(
                line.len() <= 40,
                "line {n} spans {} columns: {line}",
                line.len()
            );
        }
        assert!(f.is_ascii());
    }
}

#[test]
fn at_the_forty_column_floor_a_counting_line_keeps_its_time_whole_and_yields_the_rate() {
    // 100.0M steps, 12.4M steps/s and elapsed 1m12s with their two
    // separators are 42 columns: ordinary values, two past the floor.
    let snap = stepping(0, 100_000_000);
    let wide = compose_frame(
        &snap,
        &FrameCtx {
            rate: 12_400_000.0,
            elapsed_secs: 72,
            ..ctx(None)
        },
    );
    assert_eq!(
        visible_lines(&wide)[2],
        "100.0M steps  12.4M steps/s  elapsed 1m12s"
    );
    let narrow = compose_frame(
        &snap,
        &FrameCtx {
            width: 40,
            rate: 12_400_000.0,
            elapsed_secs: 72,
            ..ctx(None)
        },
    );
    assert_eq!(
        visible_lines(&narrow)[2],
        "100.0M steps  elapsed 1m12s",
        "a clip would have left `elapsed 1m1`"
    );
    // Two columns wider and all three fit again.
    let fits = compose_frame(
        &snap,
        &FrameCtx {
            width: 42,
            rate: 12_400_000.0,
            elapsed_secs: 72,
            ..ctx(None)
        },
    );
    assert_eq!(
        visible_lines(&fits)[2],
        "100.0M steps  12.4M steps/s  elapsed 1m12s"
    );
}

#[test]
fn a_plain_counting_line_carries_the_tally_the_rate_and_the_elapsed_time() {
    let snap = stepping(0, 43_040);
    assert_eq!(
        plain_counting_line(&snap, &BOOT, 1_200_000.0, 12),
        "[boot] stepping  43.0k steps  1.2M steps/s  elapsed 12s"
    );
    assert_eq!(
        plain_counting_line(&snap, &BOOT, 0.0, 75),
        "[boot] stepping  43.0k steps  elapsed 1m15s"
    );
    let named = Snapshot {
        current: "sys/external/liblv2.sprx".to_string(),
        ..snap
    };
    assert_eq!(
        plain_counting_line(&named, &BOOT, 0.0, 0),
        "[boot] stepping  43.0k steps  elapsed 0s  sys/external/liblv2.sprx"
    );
}

#[test]
fn a_plain_line_past_its_finish_line_shows_the_true_count() {
    assert_eq!(
        plain_line(&stepping(40_000, 44_100), &BOOT, 1.0),
        "[boot] stepping  100%  44.1k / 40.0k"
    );
}

#[test]
fn only_the_measured_phase_without_a_denominator_counts() {
    assert!(counting(&stepping(0, 0), &BOOT));
    assert!(!counting(&stepping(40_000, 0), &BOOT));
    assert!(!counting(
        &Snapshot {
            phase: LOADING,
            ..stepping(0, 0)
        },
        &BOOT
    ));
}

#[test]
fn every_unit_tallies_with_its_noun() {
    assert_eq!(Unit::Steps.tally(43_040), "43.0k steps");
    assert_eq!(Unit::Items.tally(7), "7 items");
    assert_eq!(Unit::Files.tally(1_500), "1.5k files");
    assert_eq!(Unit::Bytes.tally(512 * 1024), "512 KiB");
}
