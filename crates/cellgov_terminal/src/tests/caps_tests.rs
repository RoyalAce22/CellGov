//! The detection ladder, driven through an injected environment.

use super::*;

struct FakeEnv {
    vars: Vec<(&'static str, &'static str)>,
    tty: bool,
    columns: Option<usize>,
    vt_opt_in: bool,
}

impl FakeEnv {
    fn tty() -> Self {
        Self {
            vars: vec![("TERM", "xterm-256color")],
            tty: true,
            columns: None,
            vt_opt_in: false,
        }
    }
    fn columns(self, columns: usize) -> Self {
        Self {
            columns: Some(columns),
            ..self
        }
    }
    fn pipe() -> Self {
        Self {
            tty: false,
            ..Self::tty()
        }
    }
    fn bare_windows() -> Self {
        Self {
            vars: vec![],
            tty: true,
            columns: None,
            vt_opt_in: true,
        }
    }
    fn with(mut self, key: &'static str, value: &'static str) -> Self {
        self.vars.retain(|(k, _)| *k != key);
        self.vars.push((key, value));
        self
    }
}

impl TermEnv for FakeEnv {
    fn var(&self, key: &str) -> Option<String> {
        self.vars
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_string())
    }
    fn stderr_is_terminal(&self) -> bool {
        self.tty
    }
    fn stderr_columns(&self) -> Option<usize> {
        self.columns
    }
    fn vt_is_opt_in(&self) -> bool {
        self.vt_opt_in
    }
}

fn mode(flags: RenderFlags, env: &FakeEnv) -> RenderMode {
    detect(flags, env).mode
}

#[test]
fn a_terminal_with_no_overrides_gets_the_full_bar() {
    let caps = detect(RenderFlags::default(), &FakeEnv::tty());
    assert_eq!(caps.mode, RenderMode::Ansi);
    assert!(caps.color);
}

#[test]
fn each_suppressing_flag_turns_the_bar_off() {
    for flags in [
        RenderFlags {
            quiet: true,
            ..Default::default()
        },
        RenderFlags {
            no_progress: true,
            ..Default::default()
        },
        RenderFlags {
            json: true,
            ..Default::default()
        },
    ] {
        let caps = detect(flags, &FakeEnv::tty());
        assert_eq!(caps.mode, RenderMode::Off, "{flags:?}");
        assert!(!caps.color, "{flags:?}");
    }
}

#[test]
fn a_redirected_stderr_or_a_dumb_terminal_drops_to_plain_lines() {
    assert_eq!(
        mode(RenderFlags::default(), &FakeEnv::pipe()),
        RenderMode::Plain
    );
    assert_eq!(
        mode(RenderFlags::default(), &FakeEnv::tty().with("TERM", "dumb")),
        RenderMode::Plain
    );
}

#[test]
fn suppressing_color_keeps_the_bar() {
    let flags = RenderFlags {
        no_color: true,
        ..Default::default()
    };
    let caps = detect(flags, &FakeEnv::tty());
    assert_eq!(caps.mode, RenderMode::Ansi);
    assert!(!caps.color);

    // NO_COLOR is honoured only when present and non-empty, per the
    // spec; an empty value is not an opt-in.
    let caps = detect(
        RenderFlags::default(),
        &FakeEnv::tty().with("NO_COLOR", "1"),
    );
    assert_eq!(caps.mode, RenderMode::Ansi);
    assert!(!caps.color);
    let caps = detect(RenderFlags::default(), &FakeEnv::tty().with("NO_COLOR", ""));
    assert!(caps.color);

    let caps = detect(
        RenderFlags::default(),
        &FakeEnv::tty().with(ENV_NO_COLOR, "1"),
    );
    assert_eq!(caps.mode, RenderMode::Ansi);
    assert!(!caps.color);
    let caps = detect(
        RenderFlags::default(),
        &FakeEnv::tty().with(ENV_NO_COLOR, ""),
    );
    assert!(caps.color);
}

#[test]
fn flags_outrank_the_environment() {
    let flags = RenderFlags {
        quiet: true,
        ..Default::default()
    };
    assert_eq!(mode(flags, &FakeEnv::tty()), RenderMode::Off);
}

#[test]
fn a_host_that_needs_no_vt_switch_animates_without_any_marker() {
    let bare = FakeEnv {
        vt_opt_in: false,
        ..FakeEnv::bare_windows()
    };
    assert_eq!(mode(RenderFlags::default(), &bare), RenderMode::Ansi);
}

#[test]
fn a_windows_console_with_no_vt_marker_falls_back_to_plain() {
    assert_eq!(
        mode(RenderFlags::default(), &FakeEnv::bare_windows()),
        RenderMode::Plain
    );
    for marker in [
        ("WT_SESSION", "1"),
        ("ConEmuANSI", "ON"),
        ("ANSICON", "120x1000 (120x30)"),
        ("TERM_PROGRAM", "vscode"),
        ("TERM", "xterm-256color"),
    ] {
        let env = FakeEnv::bare_windows().with(marker.0, marker.1);
        assert_eq!(
            mode(RenderFlags::default(), &env),
            RenderMode::Ansi,
            "{marker:?}"
        );
    }
    // An empty value is not a marker, and ConEmu's marker counts only
    // when it reads `ON`.
    for absent in [("WT_SESSION", ""), ("ANSICON", ""), ("ConEmuANSI", "OFF")] {
        let env = FakeEnv::bare_windows().with(absent.0, absent.1);
        assert_eq!(
            mode(RenderFlags::default(), &env),
            RenderMode::Plain,
            "{absent:?}"
        );
    }
}

#[test]
fn forcing_ansi_answers_for_a_console_that_exports_no_marker() {
    let flags = RenderFlags {
        force_ansi: true,
        ..Default::default()
    };
    assert_eq!(mode(flags, &FakeEnv::bare_windows()), RenderMode::Ansi);
    assert_eq!(
        mode(
            RenderFlags::default(),
            &FakeEnv::bare_windows().with(ENV_FORCE_ANSI, "1")
        ),
        RenderMode::Ansi
    );
    for off in ["", "0", "false", "no", "OFF", " off "] {
        assert_eq!(
            mode(
                RenderFlags::default(),
                &FakeEnv::bare_windows().with(ENV_FORCE_ANSI, off)
            ),
            RenderMode::Plain,
            "{off:?}"
        );
    }
}

#[test]
fn forcing_ansi_does_not_outrank_the_rungs_above_the_marker_check() {
    let forced = RenderFlags {
        force_ansi: true,
        ..Default::default()
    };
    let piped = FakeEnv {
        tty: false,
        ..FakeEnv::bare_windows()
    };
    assert_eq!(mode(forced, &piped), RenderMode::Plain);
    assert_eq!(
        mode(forced, &FakeEnv::bare_windows().with("TERM", "dumb")),
        RenderMode::Plain
    );
    assert_eq!(
        mode(
            RenderFlags {
                quiet: true,
                ..forced
            },
            &FakeEnv::bare_windows()
        ),
        RenderMode::Off
    );
}

fn width(env: &FakeEnv) -> usize {
    detect(RenderFlags::default(), env).width
}

#[test]
fn width_comes_from_columns_and_is_clamped() {
    let widths = [
        ("10", 40usize),
        ("100", 100),
        ("9000", 200),
        ("wide", 80),
        ("0", 80),
    ];
    for (set, expected) in widths {
        assert_eq!(
            width(&FakeEnv::tty().with("COLUMNS", set)),
            expected,
            "COLUMNS={set}"
        );
    }
    assert_eq!(width(&FakeEnv::tty()), 80);
}

#[test]
fn width_comes_from_the_terminal_query_when_columns_is_unset() {
    for (queried, expected) in [(10usize, 40usize), (132, 132), (9000, 200)] {
        assert_eq!(
            width(&FakeEnv::tty().columns(queried)),
            expected,
            "queried {queried}"
        );
    }
    // A console that reports no window is not a 40-column terminal.
    assert_eq!(width(&FakeEnv::tty().columns(0)), 80);
    // The query answers for every mode, so a plain or silent run sizes
    // the same as the bar would have.
    assert_eq!(width(&FakeEnv::pipe().columns(132)), 132);
    let quiet = RenderFlags {
        quiet: true,
        ..Default::default()
    };
    assert_eq!(detect(quiet, &FakeEnv::tty().columns(132)).width, 132);
}

#[test]
fn a_numeric_columns_outranks_the_terminal_query() {
    assert_eq!(
        width(&FakeEnv::tty().columns(132).with("COLUMNS", "100")),
        100
    );
    // An unparseable or zero value is no override, so the query still
    // answers.
    for unset in ["wide", "0"] {
        assert_eq!(
            width(&FakeEnv::tty().columns(132).with("COLUMNS", unset)),
            132,
            "COLUMNS={unset}"
        );
    }
}

#[test]
fn capping_at_plain_leaves_off_alone_and_strips_color() {
    let ansi = detect(RenderFlags::default(), &FakeEnv::tty());
    let capped = ansi.capped_at_plain();
    assert_eq!(capped.mode, RenderMode::Plain);
    assert!(!capped.color);
    assert_eq!(capped.width, ansi.width);

    let off = detect(
        RenderFlags {
            quiet: true,
            ..Default::default()
        },
        &FakeEnv::tty(),
    );
    assert_eq!(off.capped_at_plain().mode, RenderMode::Off);

    let plain = detect(RenderFlags::default(), &FakeEnv::pipe());
    assert_eq!(plain.capped_at_plain().mode, RenderMode::Plain);
}
