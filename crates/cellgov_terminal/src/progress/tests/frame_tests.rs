//! Frame composition, bar fill, and formatting: pure functions
//! asserted byte-for-byte, no TTY involved.

use super::*;
use crate::progress::task::Unit;

/// The install task, mirrored here so the frames these tests assert
/// are the frames the installers produce.
const INSTALL: Task = Task {
    verb: "Installing",
    tag: "install",
    phases: &[
        "reading",
        "staging",
        "verifying decrypt",
        "clearing old install",
        "committing",
        "hashing source",
        "clearing staging",
    ],
    measured: 1,
    unit: Unit::Bytes,
    items: "files",
    streaming: false,
};

const STAGING: u8 = 1;
const PROVING: u8 = 2;
const CLEARING: u8 = 3;
const COMMITTING: u8 = 4;

#[test]
fn bar_fill_is_exact_width_and_monotonic() {
    for width in [10usize, 24, 40, 77] {
        let mut prev_filled = 0usize;
        for step in 0..=100 {
            let ratio = f64::from(step) / 100.0;
            let bar = bar_fill(width, ratio);
            assert_eq!(bar.len(), width, "width {width} ratio {ratio}");
            assert!(bar.is_ascii(), "ASCII only");
            let filled = bar.chars().filter(|&c| c == '=' || c == '>').count();
            assert!(
                filled >= prev_filled,
                "fill went backwards at width {width} ratio {ratio}"
            );
            prev_filled = filled;
        }
        assert_eq!(bar_fill(width, 0.0), ".".repeat(width));
        assert_eq!(bar_fill(width, 1.0), "=".repeat(width));
        // Out-of-range ratios clamp instead of panicking or overflowing.
        assert_eq!(bar_fill(width, -1.0), ".".repeat(width));
        assert_eq!(bar_fill(width, 2.0), "=".repeat(width));
    }
}

#[test]
fn fmt_secs_reads_as_seconds_then_minutes_then_hours() {
    assert_eq!(fmt_secs(0), "0s");
    assert_eq!(fmt_secs(47), "47s");
    assert_eq!(fmt_secs(59), "59s");
    assert_eq!(fmt_secs(60), "1m00s");
    assert_eq!(fmt_secs(72), "1m12s");
    assert_eq!(fmt_secs(600), "10m00s");
    assert_eq!(fmt_secs(3599), "59m59s");
    assert_eq!(fmt_secs(3600), "1h00m");
    assert_eq!(fmt_secs(7500), "2h05m");
}

#[test]
fn fmt_secs_holds_six_columns_and_reads_off_scale_past_its_ceiling() {
    assert_eq!(fmt_secs(ETA_CEILING_SECS - 1), "99h59m");
    assert_eq!(fmt_secs(ETA_CEILING_SECS), ETA_OFF_SCALE);
    // 66666667 s is a 100M-step run at just over one step per second.
    // u64::MAX is what the saturating cast produces at the extreme.
    assert_eq!(fmt_secs(66_666_667), ETA_OFF_SCALE);
    assert_eq!(fmt_secs(u64::MAX), ETA_OFF_SCALE);
    // An edited ceiling must not leave the off-scale string false.
    assert_eq!(ETA_OFF_SCALE, format!(">{}h", ETA_CEILING_SECS / 3600 - 1));
    // The sweep covers every second through both form transitions,
    // then the boundaries above them.
    let range = (0..7200).chain([359_998, 359_999, 360_000, u64::MAX]);
    for secs in range {
        let e = fmt_secs(secs);
        assert!(e.len() <= 6, "{secs}s renders {} columns: {e}", e.len());
        assert!(e.is_ascii(), "{secs}s renders non-ASCII: {e}");
    }
}

#[test]
fn elide_keeps_the_tail_and_stays_within_budget() {
    let p = "PS3_GAME/USRDIR/BUILD/MAIN/SOUND1/SPEECH/ENGLISH.PSARC";
    let e = elide(p, 30);
    assert!(e.len() <= 30, "{} bytes", e.len());
    assert!(e.contains("..."));
    assert!(e.ends_with("ENGLISH.PSARC"), "tail kept: {e}");
    assert_eq!(elide("SHORT.BIN", 30), "SHORT.BIN");
    assert_eq!(elide("ABCDEFGH", 3), "...");
}

#[test]
fn elide_never_exceeds_its_budget() {
    let p = "PS3_GAME/USRDIR/BUILD/MAIN/SOUND1/SPEECH/ENGLISH/LONG.PSARC";
    for max in 0..=p.len() + 2 {
        let e = elide(p, max);
        assert!(e.len() <= max, "max {max}: {e:?} is {} bytes", e.len());
        if max >= p.len() {
            assert_eq!(e, p);
        } else if max >= 4 {
            assert_eq!(e.len(), max, "budget fully used at {max}");
            assert!(e.ends_with(&p[p.len() - 1..]));
        }
    }
    assert_eq!(elide("ABCDEFGH", 0), "");
    assert_eq!(elide("ABCDEFGH", 2), "..");
    // Multibyte input cuts on char boundaries instead of panicking.
    let u = "\u{4e2d}\u{6587}/\u{65e5}\u{672c}/\u{d55c}\u{ad6d}/FILE.BIN";
    for max in 0..=u.len() {
        let e = elide(u, max);
        assert!(e.len() <= max, "max {max}: {e:?}");
    }
}

fn frame_ctx(color: bool) -> FrameCtx<'static> {
    FrameCtx {
        task: &INSTALL,
        label: "x.iso",
        width: 80,
        color,
        first: true,
        ratio: 0.5,
        rate: 0.0,
        eta: None,
        elapsed_secs: 0,
        spinner: '|',
        done: false,
    }
}

/// The frame with every CSI (`ESC [ ... final`) and OSC (`ESC ] ...
/// BEL`) sequence removed: what occupies columns on the terminal.
pub(super) fn visible(frame: &str) -> String {
    let mut out = String::new();
    let mut it = frame.chars();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                for c in it.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                for c in it.by_ref() {
                    if c == '\x07' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub(super) fn visible_lines(frame: &str) -> Vec<String> {
    let v = visible(frame);
    v.lines().map(str::to_string).collect()
}

fn staging_snapshot() -> Snapshot {
    Snapshot {
        phase: STAGING,
        total_amount: 1 << 30,
        done_amount: 1 << 29,
        preset_amount: 0,
        total_items: 100,
        done_items: 50,
        current: "PS3_GAME/USRDIR/EBOOT.BIN".to_string(),
    }
}

#[test]
fn frames_are_synchronized_ascii_and_line_stable() {
    let snap = staging_snapshot();
    for first in [true, false] {
        let f = compose_frame(
            &snap,
            &FrameCtx {
                first,
                ..frame_ctx(false)
            },
        );
        assert!(f.starts_with("\x1b[?2026h"), "opens synchronized update");
        assert!(f.ends_with("\x1b[?2026l"), "closes synchronized update");
        assert_eq!(
            f.matches('\n').count(),
            3,
            "exactly three lines, always, so the cursor-up arithmetic holds"
        );
        assert_eq!(
            f.contains("\x1b[3A"),
            !first,
            "cursor-up on every frame after the first"
        );
        assert!(f.is_ascii(), "no non-ASCII glyphs in a frame");
        assert!(f.contains(" 50%"), "integer percent rendered: {f}");
        assert!(f.contains("512.0 MiB / 1.00 GiB"), "byte stats: {f}");
    }
}

#[test]
fn the_install_frame_renders_its_three_lines_verbatim() {
    let f = compose_frame(&staging_snapshot(), &frame_ctx(false));
    assert_eq!(
        visible_lines(&f),
        vec![
            "Installing x.iso  [staging]".to_string(),
            "[========================>..........................]  50%  512.0 MiB / 1.00 GiB"
                .to_string(),
            "50/100 files  PS3_GAME/USRDIR/EBOOT.BIN".to_string(),
        ]
    );
}

#[test]
fn colorless_frames_carry_no_sgr() {
    let snap = staging_snapshot();
    let plain = compose_frame(&snap, &frame_ctx(false));
    assert!(!plain.contains("\x1b[1m") && !plain.contains("\x1b[2m"));
    let colored = compose_frame(&snap, &frame_ctx(true));
    assert!(colored.contains("\x1b[1m") && colored.contains("\x1b[0m"));
}

#[test]
fn indeterminate_phases_render_a_spinner_not_a_bar() {
    let snap = Snapshot {
        phase: PROVING,
        ..staging_snapshot()
    };
    let f = compose_frame(
        &snap,
        &FrameCtx {
            spinner: '/',
            ratio: 0.9,
            ..frame_ctx(false)
        },
    );
    assert!(f.contains("/ verifying decrypt..."), "{f}");
    assert!(!f.contains('%'), "no fake percentage: {f}");
}

#[test]
fn every_visible_line_fits_the_width_at_40_and_200() {
    // An NPDRM package name's shape and length -- 55 chars, enough
    // that line 1 alone passes 80 columns once "Installing " and
    // "  [verifying decrypt]" join it.
    let label = "XX0000-ABCD12345_00-EXAMPLETITLE0000-A0101-V0100-XX.pkg";
    let snaps = [
        staging_snapshot(),
        Snapshot {
            total_amount: u64::MAX,
            done_amount: u64::MAX - 1,
            total_items: 123_456,
            done_items: 123_455,
            current: "PS3_GAME/USRDIR/BUILD/MAIN/SOUND1/SPEECH/ENGLISH/VERY_LONG_NAME.PSARC"
                .to_string(),
            ..staging_snapshot()
        },
        Snapshot {
            phase: CLEARING,
            ..staging_snapshot()
        },
    ];
    for width in [40usize, 80, 200] {
        for snap in &snaps {
            for (rate, eta) in [(0.0, None), (1023.9 * 1024.0 * 1024.0, Some(59_999))] {
                for done in [false, true] {
                    let f = compose_frame(
                        snap,
                        &FrameCtx {
                            label,
                            width,
                            color: true,
                            first: false,
                            ratio: 0.999,
                            rate,
                            eta,
                            spinner: '-',
                            done,
                            ..frame_ctx(true)
                        },
                    );
                    let lines = visible_lines(&f);
                    assert_eq!(lines.len(), 3, "width {width}: {f:?}");
                    for l in &lines {
                        assert!(
                            l.len() <= width,
                            "width {width}: line of {} columns: {l:?}",
                            l.len()
                        );
                    }
                    assert!(f.is_ascii());
                }
            }
        }
    }
}

#[test]
fn every_consumer_task_fits_the_forty_column_floor() {
    const TASKS: [Task; 4] = [
        INSTALL,
        Task {
            verb: "Downloading",
            tag: "fetch",
            phases: &["resolving", "downloading", "verifying digest", "installing"],
            measured: 1,
            unit: Unit::Bytes,
            items: "",
            streaming: false,
        },
        Task {
            verb: "Booting",
            tag: "boot",
            phases: &["loading", "stepping", "summarizing"],
            measured: 1,
            unit: Unit::Steps,
            items: "",
            streaming: false,
        },
        Task {
            verb: "Migrating",
            tag: "migrate",
            phases: &[
                "planning",
                "moving firmware",
                "moving titles",
                "rewriting records",
            ],
            measured: 2,
            unit: Unit::Files,
            items: "titles",
            streaming: false,
        },
    ];
    let label = "XX0000-ABCD12345_00-EXAMPLETITLE0000-A0101-V0100-XX.pkg";
    for task in &TASKS {
        for phase in 0..task.phases.len() as u8 {
            for done in [false, true] {
                for width in [40usize, 80] {
                    let snap = Snapshot {
                        phase,
                        ..staging_snapshot()
                    };
                    let f = compose_frame(
                        &snap,
                        &FrameCtx {
                            task,
                            label,
                            width,
                            rate: 4_000_000.0,
                            eta: Some(59_999),
                            done,
                            ..frame_ctx(true)
                        },
                    );
                    let lines = visible_lines(&f);
                    assert_eq!(lines.len(), 3, "{} phase {phase}", task.verb);
                    for l in &lines {
                        assert!(
                            l.len() <= width,
                            "{} phase {phase} width {width}: {} columns: {l:?}",
                            task.verb,
                            l.len()
                        );
                    }
                    assert!(f.is_ascii());
                }
            }
        }
    }
}

#[test]
fn a_non_ascii_label_or_path_still_renders_an_ascii_frame() {
    let snap = Snapshot {
        current: "PS3_GAME/USRDIR/caf\u{e9}\n\u{4e2d}.BIN".to_string(),
        ..staging_snapshot()
    };
    let f = compose_frame(
        &snap,
        &FrameCtx {
            label: "fl\u{d6}w \u{2013} copy.pkg",
            ..frame_ctx(false)
        },
    );
    assert!(f.is_ascii(), "{f:?}");
    let lines = visible_lines(&f);
    assert_eq!(
        lines.len(),
        3,
        "a control byte in a path must not add a line"
    );
    assert!(
        lines[0].contains("Installing fl?w ? copy.pkg"),
        "{}",
        lines[0]
    );
    assert!(lines[2].ends_with("caf???.BIN"), "{}", lines[2]);
}

#[test]
fn the_final_frame_reads_done_at_one_hundred_percent() {
    // At teardown the phase is past the measured one, so only `done`
    // keeps line 2 a bar.
    let snap = Snapshot {
        phase: COMMITTING,
        done_amount: 1 << 30,
        ..staging_snapshot()
    };
    let f = compose_frame(
        &snap,
        &FrameCtx {
            ratio: 1.0,
            spinner: '=',
            done: true,
            ..frame_ctx(false)
        },
    );
    let lines = visible_lines(&f);
    assert!(lines[0].ends_with("[done]"), "{}", lines[0]);
    assert!(lines[1].contains("100%"), "{}", lines[1]);
    assert!(!lines[1].contains("committing"), "{}", lines[1]);
    let bar_w = lines[1].find(']').expect("bar closes") - 1;
    assert_eq!(&lines[1][1..=bar_w], &"=".repeat(bar_w));

    // No denominator at all: still no spinner left behind.
    let empty = Snapshot {
        total_amount: 0,
        done_amount: 0,
        ..snap
    };
    let f = compose_frame(
        &empty,
        &FrameCtx {
            ratio: 1.0,
            spinner: '=',
            done: true,
            ..frame_ctx(false)
        },
    );
    assert_eq!(visible_lines(&f)[1], "= done");
}

#[test]
fn a_measured_phase_with_unknown_totals_renders_a_spinner_not_a_bar() {
    let snap = Snapshot {
        total_amount: 0,
        done_amount: 0,
        ..staging_snapshot()
    };
    let f = compose_frame(&snap, &frame_ctx(false));
    assert!(f.contains("| staging..."), "{f}");
    assert!(!f.contains('%'), "no percent of zero: {f}");
}

#[test]
fn a_counting_unit_formats_the_totals_the_rate_and_the_eta() {
    const BENCH: Task = Task {
        verb: "Booting",
        tag: "boot",
        phases: &["loading", "stepping", "summarizing"],
        measured: 1,
        unit: Unit::Steps,
        items: "",
        streaming: false,
    };
    let snap = Snapshot {
        phase: STAGING,
        total_amount: 100_000_000,
        done_amount: 25_000_000,
        preset_amount: 0,
        total_items: 0,
        done_items: 0,
        current: String::new(),
    };
    let f = compose_frame(
        &snap,
        &FrameCtx {
            task: &BENCH,
            label: "synthetic",
            ratio: 0.25,
            rate: 12_400_000.0,
            eta: Some(72),
            ..frame_ctx(false)
        },
    );
    let lines = visible_lines(&f);
    assert_eq!(lines[0], "Booting synthetic  [stepping]");
    assert!(lines[1].contains(" 25%  25.0M / 100.0M"), "{}", lines[1]);
    // No item counter, so line 3 opens on the rate rather than on a
    // stray separator.
    assert_eq!(lines[2], "12.4M steps/s  ETA 1m12s");
}

/// A step rate just above [`Unit::eta_rate_floor`] over a 100M-step
/// run predicts about 1e8 seconds.
#[test]
fn a_stalled_counting_run_keeps_the_item_name_beside_an_off_scale_eta() {
    const BENCH: Task = Task {
        verb: "Booting",
        tag: "boot",
        phases: &["loading", "stepping"],
        measured: 1,
        unit: Unit::Steps,
        items: "",
        streaming: false,
    };
    let snap = Snapshot {
        phase: STAGING,
        total_amount: 100_000_000,
        done_amount: 0,
        preset_amount: 0,
        total_items: 0,
        done_items: 0,
        current: "sys/external/liblv2.sprx".to_string(),
    };
    let f = compose_frame(
        &snap,
        &FrameCtx {
            task: &BENCH,
            label: "synthetic",
            width: 40,
            ratio: 0.0,
            rate: 1.5,
            eta: Some(66_666_667),
            ..frame_ctx(false)
        },
    );
    let lines = visible_lines(&f);
    assert_eq!(lines[2], "1 steps/s  ETA >99h  sys/e...liblv2.sprx");
    for (n, line) in lines.iter().enumerate() {
        assert!(line.len() <= 40, "line {n} spans {} columns", line.len());
    }
}

#[test]
fn a_resumed_transfer_opens_at_its_preset_ratio() {
    const FETCH: Task = Task {
        verb: "Downloading",
        tag: "fetch",
        phases: &["resolving", "downloading", "verifying digest"],
        measured: 1,
        unit: Unit::Bytes,
        items: "",
        streaming: false,
    };
    let snap = Snapshot {
        phase: STAGING,
        total_amount: 1000,
        done_amount: 400,
        preset_amount: 400,
        total_items: 0,
        done_items: 0,
        current: "PS3UPDAT.PUP".to_string(),
    };
    let f = compose_frame(
        &snap,
        &FrameCtx {
            task: &FETCH,
            label: "4.90",
            ratio: 0.4,
            ..frame_ctx(false)
        },
    );
    let lines = visible_lines(&f);
    assert!(lines[1].contains(" 40%  400 B / 1000 B"), "{}", lines[1]);
    assert!(
        lines[1].starts_with("[==="),
        "the bar opens filled, not empty: {}",
        lines[1]
    );
}

#[test]
fn plain_threshold_lines_carry_the_tag_the_item_and_drop_an_absent_item_counter() {
    assert_eq!(
        plain_line(&staging_snapshot(), &INSTALL, 0.5),
        "[install] staging   50%  512.0 MiB / 1.00 GiB  (50/100 files)  \
         PS3_GAME/USRDIR/EBOOT.BIN"
    );

    const BENCH: Task = Task {
        verb: "Booting",
        tag: "boot",
        phases: &["loading", "stepping"],
        measured: 1,
        unit: Unit::Steps,
        items: "",
        streaming: true,
    };
    let snap = Snapshot {
        phase: STAGING,
        total_amount: 100_000_000,
        done_amount: 30_000_000,
        current: String::new(),
        ..staging_snapshot()
    };
    assert_eq!(
        plain_line(&snap, &BENCH, 0.3),
        "[boot] stepping   30%  30.0M / 100.0M"
    );
}

#[test]
fn an_indeterminate_plain_line_names_its_phase_and_item_instead_of_a_ratio() {
    const ANCHORS: Task = Task {
        verb: "Recording",
        tag: "anchors",
        phases: &["measuring"],
        measured: 0,
        unit: Unit::Items,
        items: "",
        streaming: true,
    };
    let snap = Snapshot {
        phase: 0,
        total_amount: 0,
        done_amount: 0,
        current: "synthetic (2/5)".to_string(),
        ..staging_snapshot()
    };
    assert_eq!(
        plain_indeterminate_line(&snap, &ANCHORS),
        "[anchors] measuring  synthetic (2/5)"
    );
    assert_eq!(
        plain_indeterminate_line(
            &Snapshot {
                current: String::new(),
                ..snap
            },
            &ANCHORS
        ),
        "[anchors] measuring"
    );
}

#[test]
fn an_empty_phase_table_fits_line_one_instead_of_wrapping_it() {
    const EMPTY: Task = Task {
        phases: &[],
        ..INSTALL
    };
    let label = "XX0000-ABCD12345_00-EXAMPLETITLE0000-A0101-V0100-XX.pkg";
    for width in [40usize, 80] {
        let f = compose_frame(
            &staging_snapshot(),
            &FrameCtx {
                task: &EMPTY,
                label,
                width,
                ..frame_ctx(false)
            },
        );
        let lines = visible_lines(&f);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].ends_with("[working]"), "{}", lines[0]);
        for l in &lines {
            assert!(
                l.len() <= width,
                "width {width}: {} columns: {l:?}",
                l.len()
            );
        }
    }
}

#[test]
fn a_verb_or_phase_label_wider_than_the_terminal_is_elided_not_wrapped() {
    const WIDE: Task = Task {
        verb: "Reticulating splines throughout the whole content package",
        phases: &[
            "reticulating every last spline in the whole content package",
            "staging",
        ],
        ..INSTALL
    };
    for width in [40usize, 80, 200] {
        for phase in [0u8, 1] {
            for done in [false, true] {
                let snap = Snapshot {
                    phase,
                    ..staging_snapshot()
                };
                let f = compose_frame(
                    &snap,
                    &FrameCtx {
                        task: &WIDE,
                        width,
                        done,
                        ..frame_ctx(true)
                    },
                );
                let lines = visible_lines(&f);
                assert_eq!(lines.len(), 3, "width {width} phase {phase}");
                for l in &lines {
                    assert!(
                        l.len() <= width,
                        "width {width} phase {phase}: {} columns: {l:?}",
                        l.len()
                    );
                }
            }
        }
    }
}

#[test]
fn an_out_of_range_ratio_cannot_print_a_percent_the_bar_contradicts() {
    assert_eq!(percent(f64::NAN), 0);
    assert_eq!(percent(f64::INFINITY), 100);
    assert_eq!(percent(f64::NEG_INFINITY), 0);
    assert_eq!(percent(-0.5), 0);
    assert_eq!(percent(1.0), 100);
    for (ratio, want) in [
        (f64::INFINITY, "100%"),
        (2.0, "100%"),
        (f64::NAN, "  0%"),
        (-1.0, "  0%"),
    ] {
        let f = compose_frame(
            &staging_snapshot(),
            &FrameCtx {
                ratio,
                ..frame_ctx(false)
            },
        );
        let lines = visible_lines(&f);
        assert!(lines[1].contains(want), "ratio {ratio}: {}", lines[1]);
        assert!(lines[1].len() <= 80, "{}", lines[1]);
    }
}

#[test]
fn an_item_name_alone_on_line_three_spends_the_whole_width() {
    const BARE: Task = Task {
        items: "",
        ..INSTALL
    };
    let snap = Snapshot {
        current: "A".repeat(200),
        ..staging_snapshot()
    };
    let f = compose_frame(
        &snap,
        &FrameCtx {
            task: &BARE,
            ..frame_ctx(false)
        },
    );
    assert_eq!(visible_lines(&f)[2].len(), 80);
}

#[test]
fn osc_progress_never_emits_a_partial_state() {
    assert_eq!(osc_progress(1, Some(52)), "\x1b]9;4;1;52\x07");
    assert_eq!(osc_progress(3, None), "\x1b]9;4;3\x07");
    assert_eq!(osc_progress(2, None), "\x1b]9;4;2\x07");
    assert_eq!(osc_progress(0, None), "\x1b]9;4;0\x07");
    // Percent is clamped so a drifting ratio cannot emit >100.
    assert_eq!(osc_progress(1, Some(140)), "\x1b]9;4;1;100\x07");
}
