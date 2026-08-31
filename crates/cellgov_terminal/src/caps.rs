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
    /// Terminal width in columns.
    ///
    /// Read from `COLUMNS`, which shells rarely export, so this is 80
    /// in practice.
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
}

impl RenderFlags {
    /// Absorb a render flag, reporting whether `arg` was one.
    ///
    /// `--format json` is a two-token flag the caller's own parser
    /// owns; it sets [`Self::json`] directly.
    pub fn accept(&mut self, arg: &str) -> bool {
        match arg {
            "--no-progress" => self.no_progress = true,
            "--no-color" => self.no_color = true,
            "--quiet" => self.quiet = true,
            _ => return false,
        }
        true
    }

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
}

fn non_empty(env: &dyn TermEnv, var: &str) -> bool {
    env.var(var).is_some_and(|v| !v.is_empty())
}

/// Whether the environment carries a marker of a VT-capable terminal.
///
/// Windows only, and a marker search rather than `SetConsoleMode`,
/// which is FFI the workspace's `forbid(unsafe_code)` rules out.
fn ansi_capable_console(env: &dyn TermEnv) -> bool {
    if !cfg!(windows) {
        return true;
    }
    non_empty(env, "WT_SESSION")
        || env.var("ConEmuANSI").is_some_and(|v| v == "ON")
        || non_empty(env, "TERM_PROGRAM")
        || non_empty(env, "TERM")
}

/// Decide the rendering mode and palette once against `env`, with the
/// flags outranking it.
#[must_use]
pub fn detect(flags: RenderFlags, env: &dyn TermEnv) -> TermCaps {
    let width = env
        .var("COLUMNS")
        .and_then(|v| v.parse::<usize>().ok())
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
    if !env.stderr_is_terminal() || dumb || !ansi_capable_console(env) {
        return TermCaps {
            mode: RenderMode::Plain,
            color: false,
            width,
        };
    }
    TermCaps {
        mode: RenderMode::Ansi,
        color: !flags.no_color && !non_empty(env, "NO_COLOR"),
        width,
    }
}

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
