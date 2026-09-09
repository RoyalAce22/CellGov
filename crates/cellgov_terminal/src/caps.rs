//! Startup capability detection: one decision, one precedence order,
//! for every command's human-facing output.

/// How progress is rendered, decided once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// No progress output at all (`--quiet` / `--no-progress` /
    /// `--format json`).
    Off,
    /// Plain threshold lines: non-TTY stderr, `TERM=dumb`, a Windows
    /// console with no ANSI marker, or a command that streams its own
    /// lines while working.
    Plain,
    /// Full in-place bar with escape sequences.
    Ansi,
}

/// The startup capability decision.
#[derive(Debug, Clone, Copy)]
pub struct TermCaps {
    /// Rendering mode.
    pub mode: RenderMode,
    /// Whether SGR color/style sequences are emitted (Ansi mode only).
    pub color: bool,
    /// Terminal width in columns, clamped to `40..=200`.
    ///
    /// A numeric `COLUMNS` outranks the queried width of the terminal
    /// on stderr, so an operator can pin the frame width; with neither,
    /// 80.
    pub width: usize,
}

impl TermCaps {
    /// Drop [`RenderMode::Ansi`] to [`RenderMode::Plain`], leaving
    /// `Off` alone.
    ///
    /// The in-place frame cursor-ups over its own lines, so any other
    /// writer scrolling the same terminal corrupts it; threshold lines
    /// interleave harmlessly.
    #[must_use]
    pub fn capped_at_plain(self) -> Self {
        match self.mode {
            RenderMode::Ansi => Self {
                mode: RenderMode::Plain,
                color: false,
                ..self
            },
            _ => self,
        }
    }
}

/// Command-line overrides, which outrank the environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderFlags {
    /// `--no-progress`: no bar, other output unaffected.
    pub no_progress: bool,
    /// `--no-color`: bar without SGR.
    pub no_color: bool,
    /// `--quiet`: no bar.
    pub quiet: bool,
    /// `--format json`: machine mode never animates.
    pub json: bool,
    /// `--force-ansi`: treat a Windows console with no VT marker as ANSI.
    pub force_ansi: bool,
}

impl RenderFlags {
    /// Resolve these flags against the process environment.
    #[must_use]
    pub fn caps(self) -> TermCaps {
        detect(self, &HostEnv)
    }
}

/// The environment reads [`detect`] makes.
pub trait TermEnv {
    /// The value of environment variable `key`, if set.
    fn var(&self, key: &str) -> Option<String>;
    /// Whether stderr is attached to a terminal.
    fn stderr_is_terminal(&self) -> bool;
    /// The column count of the terminal on stderr, if it is one.
    fn stderr_columns(&self) -> Option<usize>;
    /// Whether a terminal must turn VT on before escape sequences work.
    fn vt_is_opt_in(&self) -> bool;
}

/// [`TermEnv`] over the real process environment.
#[derive(Debug, Clone, Copy)]
pub struct HostEnv;

impl TermEnv for HostEnv {
    fn var(&self, key: &str) -> Option<String> {
        // Lossy rather than `env::var`, which reports a value that is
        // not UTF-8 as absent: `NO_COLOR` and the VT markers are
        // presence checks, and a set-but-unreadable value must not read
        // as unset.
        std::env::var_os(key).map(|v| v.to_string_lossy().into_owned())
    }
    fn stderr_is_terminal(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stderr())
    }
    fn stderr_columns(&self) -> Option<usize> {
        // The bar renders on stderr, which can be a different terminal
        // from stdout, so the query reads stderr.
        terminal_size::terminal_size_of(std::io::stderr()).map(|(w, _)| usize::from(w.0))
    }
    fn vt_is_opt_in(&self) -> bool {
        cfg!(windows)
    }
}

fn non_empty(env: &dyn TermEnv, var: &str) -> bool {
    env.var(var).is_some_and(|v| !v.is_empty())
}

/// Whether `var` is set to anything but an off value.
///
/// Reads the off tokens the CLI's strict env parse accepts, so
/// `CELLGOV_FORCE_ANSI=0` leaves the override off.
fn env_enables(env: &dyn TermEnv, var: &str) -> bool {
    env.var(var).is_some_and(|v| {
        !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        )
    })
}

/// Whether a Windows console processes VT sequences.
///
/// `SetConsoleMode` is both the query and the switch, and it is FFI that
/// `forbid(unsafe_code)` rules out. So this searches for markers a host
/// left when it made escape sequences work. A bare `conhost` exports no
/// marker, so [`ENV_FORCE_ANSI`] is how an operator answers for it.
fn ansi_capable_console(flags: RenderFlags, env: &dyn TermEnv) -> bool {
    if !env.vt_is_opt_in() {
        return true;
    }
    flags.force_ansi
        || env_enables(env, ENV_FORCE_ANSI)
        // Each marker names a host that makes escape sequences work:
        // - Windows Terminal and ConEmu turn VT on.
        // - ANSICON's injected DLL interprets the sequences itself.
        // - `TERM_PROGRAM` and `TERM` name the VS Code and MSYS hosts.
        || non_empty(env, "WT_SESSION")
        || env.var("ConEmuANSI").is_some_and(|v| v == "ON")
        || non_empty(env, "ANSICON")
        || non_empty(env, "TERM_PROGRAM")
        || non_empty(env, "TERM")
}

/// Decide the rendering mode and palette once against `env`, with the
/// flags outranking it.
#[must_use]
pub fn detect(flags: RenderFlags, env: &dyn TermEnv) -> TermCaps {
    // `COLUMNS` comes first, as the terminal libraries read it: it is
    // the operator's override. A zero from either source is not a width
    // (an unset override, or a console that reports no window), so it
    // falls through like an absent value.
    let width = env
        .var("COLUMNS")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&w| w > 0)
        .or_else(|| env.stderr_columns().filter(|&w| w > 0))
        .unwrap_or(80)
        .clamp(40, 200);
    if flags.quiet || flags.no_progress || flags.json {
        return TermCaps {
            mode: RenderMode::Off,
            color: false,
            width,
        };
    }
    let dumb = env.var("TERM").is_some_and(|t| t == "dumb");
    if !env.stderr_is_terminal() || dumb || !ansi_capable_console(flags, env) {
        return TermCaps {
            mode: RenderMode::Plain,
            color: false,
            width,
        };
    }
    TermCaps {
        mode: RenderMode::Ansi,
        // `NO_COLOR` is the cross-tool convention; `CELLGOV_NO_COLOR`
        // scopes the same decision to this program, for an operator who
        // wants color everywhere else.
        color: !flags.no_color && !non_empty(env, "NO_COLOR") && !non_empty(env, ENV_NO_COLOR),
        width,
    }
}

/// App-scoped companion to `NO_COLOR`.
pub const ENV_NO_COLOR: &str = "CELLGOV_NO_COLOR";

/// Env-var form of `--force-ansi`.
///
/// This one honours `0`, `false`, `no`, and `off`; [`ENV_NO_COLOR`]
/// follows `NO_COLOR` and reads any non-empty value as set.
pub const ENV_FORCE_ANSI: &str = "CELLGOV_FORCE_ANSI";

/// SGR helpers that collapse to nothing when color is off.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// Whether to emit sequences at all.
    pub on: bool,
}

impl Style {
    /// Bold.
    #[must_use]
    pub fn bold(&self) -> &'static str {
        if self.on {
            "\x1b[1m"
        } else {
            ""
        }
    }
    /// Dim.
    #[must_use]
    pub fn dim(&self) -> &'static str {
        if self.on {
            "\x1b[2m"
        } else {
            ""
        }
    }
    /// Back to the terminal's default attributes.
    #[must_use]
    pub fn reset(&self) -> &'static str {
        if self.on {
            "\x1b[0m"
        } else {
            ""
        }
    }
}

#[cfg(test)]
#[path = "tests/caps_tests.rs"]
mod tests;
