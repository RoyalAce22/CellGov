//! Frame composition, bar fill, and formatting: pure functions
//! asserted byte-for-byte, no TTY involved.

use super::*;

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
fn fmt_bytes_picks_the_right_unit() {
    assert_eq!(fmt_bytes(97), "97 B");
    assert_eq!(fmt_bytes(512 * 1024), "512 KiB");
    assert_eq!(fmt_bytes(38 * 1024 * 1024 + 200 * 1024), "38.2 MiB");
    assert_eq!(fmt_bytes(2 * 1024 * 1024 * 1024), "2.00 GiB");
}

#[test]
fn fmt_eta_reads_as_minutes_and_seconds() {
    assert_eq!(fmt_eta(47), "47s");
    assert_eq!(fmt_eta(72), "1m12s");
    assert_eq!(fmt_eta(600), "10m00s");
}

#[test]
fn elide_path_keeps_the_filename_and_stays_within_budget() {
    let p = "PS3_GAME/USRDIR/BUILD/MAIN/SOUND1/SPEECH/ENGLISH.PSARC";
    let e = elide_path(p, 30);
    assert!(e.len() <= 30, "{} bytes", e.len());
    assert!(e.contains("..."));
    assert!(e.ends_with("ENGLISH.PSARC"), "tail kept: {e}");
    assert_eq!(elide_path("SHORT.BIN", 30), "SHORT.BIN");
    assert_eq!(elide_path("ABCDEFGH", 3), "...");
}

fn frame_ctx(color: bool) -> FrameCtx<'static> {
    FrameCtx {
        label: "x.iso",
        width: 80,
        color,
        first: true,
        ratio: 0.5,
        rate: 0.0,
        eta: None,
        spinner: '|',
        done: false,
    }
}

/// The frame with every CSI (`ESC [ ... final`) and OSC (`ESC ] ...
/// BEL`) sequence removed: what occupies columns on the terminal.
fn visible(frame: &str) -> String {
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

fn visible_lines(frame: &str) -> Vec<String> {
    let v = visible(frame);
    v.lines().map(str::to_string).collect()
}

fn staging_snapshot() -> Snapshot {
    Snapshot {
        phase: Phase::Staging,
        total_bytes: 1 << 30,
        done_bytes: 1 << 29,
        total_files: 100,
        done_files: 50,
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
                label: "x.iso",
                width: 80,
                color: false,
                first,
                ratio: 0.5,
                rate: 0.0,
                eta: None,
                spinner: '|',
                done: false,
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
        phase: Phase::Proving,
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
    // A real NPDRM package name: 55 chars, so line 1 alone would pass
    // 80 columns once "Installing " and "  [verifying decrypt]" join
    // it, and the cursor-up-by-three of the next frame would land a
    // row low.
    let label = "UP9000-NPUA80001_00-FLOWFLOWFLOW0000-A0101-V0100-PE.pkg";
    let snaps = [
        staging_snapshot(),
        Snapshot {
            total_bytes: u64::MAX,
            done_bytes: u64::MAX - 1,
            total_files: 123_456,
            done_files: 123_455,
            current: "PS3_GAME/USRDIR/BUILD/MAIN/SOUND1/SPEECH/ENGLISH/VERY_LONG_NAME.PSARC"
                .to_string(),
            ..staging_snapshot()
        },
        Snapshot {
            phase: Phase::Clearing,
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
    // At teardown the phase is Committing; without `done` line 2
    // would be a `committing...` spinner and the 1.0 ratio unused.
    let snap = Snapshot {
        phase: Phase::Committing,
        done_bytes: 1 << 30,
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

    // No byte denominator at all: still no spinner left behind.
    let empty = Snapshot {
        total_bytes: 0,
        done_bytes: 0,
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
fn staging_with_unknown_totals_renders_a_spinner_not_a_bar() {
    let snap = Snapshot {
        total_bytes: 0,
        done_bytes: 0,
        ..staging_snapshot()
    };
    let f = compose_frame(&snap, &frame_ctx(false));
    assert!(f.contains("| staging..."), "{f}");
    assert!(!f.contains('%'), "no percent of zero: {f}");
}

#[test]
fn elide_path_never_exceeds_its_budget() {
    let p = "PS3_GAME/USRDIR/BUILD/MAIN/SOUND1/SPEECH/ENGLISH/LONG.PSARC";
    for max in 0..=p.len() + 2 {
        let e = elide_path(p, max);
        assert!(e.len() <= max, "max {max}: {e:?} is {} bytes", e.len());
        if max >= p.len() {
            assert_eq!(e, p);
        } else if max >= 4 {
            assert_eq!(e.len(), max, "budget fully used at {max}");
            assert!(e.ends_with(&p[p.len() - 1..]));
        }
    }
    assert_eq!(elide_path("ABCDEFGH", 0), "");
    assert_eq!(elide_path("ABCDEFGH", 2), "..");
    // Multibyte input cuts on char boundaries instead of panicking.
    let u = "\u{4e2d}\u{6587}/\u{65e5}\u{672c}/\u{d55c}\u{ad6d}/FILE.BIN";
    for max in 0..=u.len() {
        let e = elide_path(u, max);
        assert!(e.len() <= max, "max {max}: {e:?}");
    }
}

#[test]
fn fmt_bytes_switches_unit_at_the_boundary_and_survives_max() {
    assert_eq!(fmt_bytes(0), "0 B");
    assert_eq!(fmt_bytes(1023), "1023 B");
    assert_eq!(fmt_bytes(1024), "1 KiB");
    assert_eq!(fmt_bytes((1 << 20) - 1), "1024 KiB");
    assert_eq!(fmt_bytes(1 << 20), "1.0 MiB");
    assert_eq!(fmt_bytes(1 << 30), "1.00 GiB");
    assert!(fmt_bytes(u64::MAX).ends_with(" GiB"));
    assert_eq!(fmt_eta(0), "0s");
    assert_eq!(fmt_eta(59), "59s");
    assert_eq!(fmt_eta(60), "1m00s");
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
