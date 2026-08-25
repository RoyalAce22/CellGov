//! Terminal renderer for install progress: capability detection, a
//! shared atomic state implementing the library's reporter trait, and
//! a timer-driven draw thread that owns stderr.
//!
//! ASCII-only output throughout (no block glyphs), per the workspace's
//! console-encoding rule. All escape output is gated on the detected
//! mode, so redirected streams get plain threshold lines and `--quiet`
//! gets nothing.

use cellgov_install::progress::{InstallProgress, Phase};
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How progress is rendered, decided once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// No progress output at all (`--quiet` / `--no-progress`).
    Off,
    /// Plain threshold lines: non-TTY stderr, `TERM=dumb`, or a
    /// Windows console with no ANSI marker.
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
    pub width: usize,
}

/// Whether `var` is present with a non-empty value.
fn env_non_empty(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty())
}

/// On Windows, whether the environment carries a marker of a
/// VT-capable terminal. `SetConsoleMode` is the real check, but it is
/// FFI this workspace's forbid(unsafe_code) rules out; on a legacy
/// console with none of these markers the renderer falls back to
/// plain lines.
fn ansi_capable_console() -> bool {
    if !cfg!(windows) {
        return true;
    }
    env_non_empty("WT_SESSION")
        || std::env::var("ConEmuANSI").is_ok_and(|v| v == "ON")
        || env_non_empty("TERM_PROGRAM")
        || env_non_empty("TERM")
}

/// Decide the rendering mode and palette once.
///
/// Precedence, flags over environment: `--quiet` / `--no-progress`
/// kill the bar; a non-TTY stderr or `TERM=dumb` drops to plain
/// lines; `--no-color`, then `NO_COLOR` (present and non-empty, per
/// the spec), suppress SGR while keeping the bar -- `NO_COLOR` covers
/// color only. `CLICOLOR_FORCE` is not read: it forces color through
/// a pipe, and this renderer's only colored output is the bar, which
/// a pipe never sees.
pub fn detect(no_progress: bool, no_color: bool, quiet: bool) -> TermCaps {
    let width = std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(80)
        .clamp(40, 200);
    if quiet || no_progress {
        return TermCaps {
            mode: RenderMode::Off,
            color: false,
            width,
        };
    }
    let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
    if !std::io::stderr().is_terminal() || dumb || !ansi_capable_console() {
        return TermCaps {
            mode: RenderMode::Plain,
            color: false,
            width,
        };
    }
    TermCaps {
        mode: RenderMode::Ansi,
        color: !no_color && !env_non_empty("NO_COLOR"),
        width,
    }
}

/// `Phase` as a storable u8 (`Reading` when unset/unknown).
fn phase_code(p: Phase) -> u8 {
    match p {
        Phase::Reading => 0,
        Phase::Staging => 1,
        Phase::Proving => 2,
        Phase::Clearing => 3,
        Phase::Committing => 4,
        Phase::Hashing => 5,
    }
}

fn code_phase(c: u8) -> Phase {
    match c {
        1 => Phase::Staging,
        2 => Phase::Proving,
        3 => Phase::Clearing,
        4 => Phase::Committing,
        5 => Phase::Hashing,
        _ => Phase::Reading,
    }
}

/// Shared install state: written by the installer thread through the
/// reporter trait, read by the render thread. Counters are `Relaxed`;
/// nothing orders on them.
pub struct ProgressState {
    phase: AtomicU8,
    total_bytes: AtomicU64,
    done_bytes: AtomicU64,
    total_files: AtomicUsize,
    done_files: AtomicUsize,
    /// Touched once per file, never per piece.
    current: Mutex<String>,
    finished: AtomicBool,
}

impl ProgressState {
    fn new() -> Self {
        Self {
            phase: AtomicU8::new(0),
            total_bytes: AtomicU64::new(0),
            done_bytes: AtomicU64::new(0),
            total_files: AtomicUsize::new(0),
            done_files: AtomicUsize::new(0),
            current: Mutex::new(String::new()),
            finished: AtomicBool::new(false),
        }
    }
}

impl InstallProgress for ProgressState {
    fn phase(&self, phase: Phase) {
        self.phase.store(phase_code(phase), Ordering::Relaxed);
    }
    fn totals(&self, files: usize, bytes: u64) {
        self.total_files.store(files, Ordering::Relaxed);
        self.total_bytes.store(bytes, Ordering::Relaxed);
    }
    fn file_started(&self, path: &str) {
        let mut cur = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cur.clear();
        cur.push_str(path);
    }
    fn bytes_advanced(&self, delta: u64) {
        self.done_bytes.fetch_add(delta, Ordering::Relaxed);
    }
    fn file_finished(&self) {
        self.done_files.fetch_add(1, Ordering::Relaxed);
    }
    fn finished(&self) {
        self.finished.store(true, Ordering::Relaxed);
    }
}

/// A point-in-time copy of the state, for pure frame composition.
struct Snapshot {
    phase: Phase,
    total_bytes: u64,
    done_bytes: u64,
    total_files: usize,
    done_files: usize,
    current: String,
}

fn snapshot(state: &ProgressState) -> Snapshot {
    Snapshot {
        phase: code_phase(state.phase.load(Ordering::Relaxed)),
        total_bytes: state.total_bytes.load(Ordering::Relaxed),
        done_bytes: state.done_bytes.load(Ordering::Relaxed),
        total_files: state.total_files.load(Ordering::Relaxed),
        done_files: state.done_files.load(Ordering::Relaxed),
        current: state
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
    }
}

fn phase_label(p: Phase) -> &'static str {
    match p {
        Phase::Reading => "reading",
        Phase::Staging => "staging",
        Phase::Proving => "verifying decrypt",
        Phase::Clearing => "clearing old install",
        Phase::Committing => "committing",
        Phase::Hashing => "hashing source",
    }
}

/// ASCII bar fill: `=` done, `>` head, `.` remaining, in `width`
/// interior columns. The sub-cell block glyphs the reference designs
/// use are non-ASCII and banned by the workspace console rule.
fn bar_fill(width: usize, ratio: f64) -> String {
    let ratio = ratio.clamp(0.0, 1.0);
    let filled = ((width as f64) * ratio).floor() as usize;
    let filled = filled.min(width);
    let mut s = String::with_capacity(width);
    for _ in 0..filled.saturating_sub(1) {
        s.push('=');
    }
    if filled > 0 {
        s.push(if filled == width { '=' } else { '>' });
    }
    for _ in filled..width {
        s.push('.');
    }
    s
}

/// `1.94 GiB` / `38.2 MiB` / `512 KiB` / `97 B`.
fn fmt_bytes(n: u64) -> String {
    const GIB: f64 = (1u64 << 30) as f64;
    const MIB: f64 = (1u64 << 20) as f64;
    const KIB: f64 = (1u64 << 10) as f64;
    let x = n as f64;
    if x >= GIB {
        format!("{:.2} GiB", x / GIB)
    } else if x >= MIB {
        format!("{:.1} MiB", x / MIB)
    } else if x >= KIB {
        format!("{:.0} KiB", x / KIB)
    } else {
        format!("{n} B")
    }
}

/// `1m12s` / `47s`.
fn fmt_eta(secs: u64) -> String {
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Middle-elide `path` to at most `max` bytes with `...`, keeping the
/// tail (the filename). Staged paths are ISO9660 / PKG tree paths,
/// which are ASCII, so byte arithmetic is display-width arithmetic;
/// `char_indices` keeps a stray non-ASCII byte from panicking the cut.
fn elide_path(path: &str, max: usize) -> String {
    if path.len() <= max {
        return path.to_string();
    }
    if max <= 3 {
        // The doc promises at most `max` bytes; a full "..." at
        // max 1 or 2 would break that and push a frame line past the
        // terminal width, wrapping it under the cursor-up arithmetic.
        return "..."[..max].to_string();
    }
    let keep = max - 3;
    let head = keep / 3;
    let tail = keep - head;
    let head_end = path
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= head)
        .last()
        .unwrap_or(0);
    let tail_start = path
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= path.len() - tail)
        .unwrap_or(path.len());
    format!("{}...{}", &path[..head_end], &path[tail_start..])
}

/// Printable ASCII only: anything else becomes `?`. Keeps the
/// module's ASCII-only frame contract when the host filename or a
/// container path carries another script or a control byte, and
/// makes byte length equal display width for the column arithmetic.
fn ascii_only(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .collect()
}

/// Truncate `s` to at most `width` bytes on a char boundary.
fn clip_columns(s: &mut String, width: usize) {
    if s.len() > width {
        let mut cut = width;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
}

const ALL_PHASES: [Phase; 6] = [
    Phase::Reading,
    Phase::Staging,
    Phase::Proving,
    Phase::Clearing,
    Phase::Committing,
    Phase::Hashing,
];

/// Longest phase label, so line 1's label budget does not change
/// (and re-elide the label) as the install moves between phases.
fn max_phase_label_len() -> usize {
    ALL_PHASES
        .iter()
        .map(|&p| phase_label(p).len())
        .max()
        .unwrap_or(0)
}

/// SGR helpers that collapse to nothing when color is off.
struct Style {
    on: bool,
}

impl Style {
    fn bold(&self) -> &'static str {
        if self.on {
            "\x1b[1m"
        } else {
            ""
        }
    }
    fn dim(&self) -> &'static str {
        if self.on {
            "\x1b[2m"
        } else {
            ""
        }
    }
    fn reset(&self) -> &'static str {
        if self.on {
            "\x1b[0m"
        } else {
            ""
        }
    }
}

/// Presentation inputs for one frame, alongside the [`Snapshot`].
struct FrameCtx<'a> {
    label: &'a str,
    width: usize,
    color: bool,
    /// First frame allocates its lines; later frames cursor-up over them.
    first: bool,
    /// Monotonic high-water fill ratio.
    ratio: f64,
    /// Smoothed bytes/second.
    rate: f64,
    eta: Option<u64>,
    spinner: char,
    /// The install completed: line 1 reads `done`, line 2 is the full
    /// bar (or `= done` when there was no byte denominator).
    done: bool,
}

/// Compose one 3-line ANSI frame into a single buffer, wrapped in DEC
/// private mode 2026 (synchronized update; terminals that lack it
/// ignore the unknown mode).
///
/// Every visible line is at most `width` columns. A line that wraps
/// occupies two terminal rows, and the next frame's cursor-up by
/// three then lands one row low, leaving a stale row behind on every
/// tick; the label and path budgets exist for that reason.
fn compose_frame(snap: &Snapshot, ctx: &FrameCtx<'_>) -> String {
    let FrameCtx {
        label,
        width,
        color,
        first,
        ratio,
        rate,
        eta,
        spinner,
        done,
    } = *ctx;
    let st = Style { on: color };
    let mut out = String::with_capacity(256);
    out.push_str("\x1b[?2026h");
    if !first {
        out.push_str("\x1b[3A");
    }

    // Line 1: label + phase. "Installing " + label + "  [" + status
    // + "]"; the label takes whatever the longest status leaves.
    let status = if done {
        "done"
    } else {
        phase_label(snap.phase)
    };
    let label_room = width.saturating_sub("Installing ".len() + 4 + max_phase_label_len());
    let label = elide_path(&ascii_only(label), label_room);
    out.push_str("\x1b[2K");
    out.push_str(&format!(
        "{}Installing {label}{}  {}[{status}]{}\n",
        st.bold(),
        st.reset(),
        st.dim(),
        st.reset(),
    ));

    // Line 2: bar or spinner.
    out.push_str("\x1b[2K");
    if snap.total_bytes > 0 && (snap.phase == Phase::Staging || done) {
        let pct = (ratio * 100.0).floor() as u32;
        let stats = format!(
            " {:>3}%  {} / {}",
            pct,
            fmt_bytes(snap.done_bytes.min(snap.total_bytes)),
            fmt_bytes(snap.total_bytes),
        );
        let bar_w = width.saturating_sub(stats.len() + 2).max(10);
        let mut line = format!("[{}]{stats}", bar_fill(bar_w, ratio));
        clip_columns(&mut line, width);
        out.push_str(&line);
        out.push('\n');
    } else if done {
        out.push_str("= done\n");
    } else {
        out.push_str(&format!("{spinner} {}...\n", phase_label(snap.phase)));
    }

    // Line 3: files, rate, ETA, current path.
    out.push_str("\x1b[2K");
    let mut line = format!("{}/{} files", snap.done_files, snap.total_files);
    if snap.phase == Phase::Staging && rate > 1.0 {
        line.push_str(&format!("  {}/s", fmt_bytes(rate as u64)));
        if let Some(e) = eta {
            line.push_str(&format!("  ETA {}", fmt_eta(e)));
        }
    }
    // The path only joins when there is room for it and its
    // separator; at width 40 with a rate and an ETA there is none.
    let room = width.saturating_sub(line.len() + 2);
    if room > 0 && !snap.current.is_empty() {
        line.push_str("  ");
        line.push_str(&elide_path(&ascii_only(&snap.current), room));
    }
    clip_columns(&mut line, width);
    out.push_str(&format!("{}{line}{}\n", st.dim(), st.reset()));

    out.push_str("\x1b[?2026l");
    out
}

/// OSC 9;4 terminal-native progress (taskbar / dock). Built by this
/// one formatter so a partial state can never be emitted.
fn osc_progress(state: u8, pct: Option<u8>) -> String {
    match pct {
        Some(p) => format!("\x1b]9;4;{state};{}\x07", p.min(100)),
        None => format!("\x1b]9;4;{state}\x07"),
    }
}

/// Marker for the panic hook: a bar is live and the cursor is hidden.
static BAR_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Install the cursor/OSC-restoring panic hook once, chained in front
/// of the previous hook so the panic message lands on a sane screen.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if BAR_ACTIVE.swap(false, Ordering::Relaxed) {
                let mut err = std::io::stderr();
                let _ = err.write_all(b"\x1b[?25h\x1b]9;4;0\x07\n");
                let _ = err.flush();
            }
            prev(info);
        }));
    });
}

/// The live progress display: owns the render thread and stderr while
/// running. Known limitation: a hard Ctrl-C kills the process without
/// unwinding, which can leave the
/// cursor hidden and a stale taskbar state -- a SIGINT handler needs
/// FFI the workspace's forbid(unsafe_code) rules out.
pub struct ProgressBar {
    state: Arc<ProgressState>,
    stop: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
    caps: TermCaps,
}

impl ProgressBar {
    /// Start rendering (a no-op shell in `Off` mode). `label` names
    /// the container being installed.
    pub fn start(caps: TermCaps, label: &str) -> Self {
        let state = Arc::new(ProgressState::new());
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let handle = match caps.mode {
            RenderMode::Off => None,
            mode => {
                install_panic_hook();
                if mode == RenderMode::Ansi {
                    BAR_ACTIVE.store(true, Ordering::Relaxed);
                }
                let st = Arc::clone(&state);
                let sp = Arc::clone(&stop);
                let label = label.to_string();
                Some(std::thread::spawn(move || {
                    render_loop(&st, &sp, caps, &label);
                }))
            }
        };
        Self {
            state,
            stop,
            handle,
            caps,
        }
    }

    /// The reporter to hand to the installers.
    pub fn state(&self) -> Arc<ProgressState> {
        Arc::clone(&self.state)
    }

    fn stop_thread(&mut self) {
        if let Some(h) = self.handle.take() {
            {
                let (lock, cvar) = &*self.stop;
                let mut stopped = lock
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *stopped = true;
                cvar.notify_all();
            }
            let _ = h.join();
        }
    }

    /// Graceful teardown after a successful install: final frame,
    /// cursor restored, taskbar state cleared.
    pub fn finish(mut self) {
        self.state.finished.store(true, Ordering::Relaxed);
        self.stop_thread();
        if self.caps.mode == RenderMode::Ansi {
            BAR_ACTIVE.store(false, Ordering::Relaxed);
            let mut err = std::io::stderr();
            let _ = err.write_all(format!("\x1b[?25h{}", osc_progress(0, None)).as_bytes());
            let _ = err.flush();
        }
    }

    /// Teardown after a failed install: error state to the taskbar,
    /// then cleared, cursor restored, bar left on screen above the
    /// error message the caller is about to print.
    pub fn abort(mut self) {
        self.stop_thread();
        if self.caps.mode == RenderMode::Ansi {
            BAR_ACTIVE.store(false, Ordering::Relaxed);
            let mut err = std::io::stderr();
            let _ = err.write_all(
                format!(
                    "\x1b[?25h{}{}",
                    osc_progress(2, None),
                    osc_progress(0, None)
                )
                .as_bytes(),
            );
            let _ = err.flush();
        }
    }
}

/// EWMA time constant for the byte rate, in seconds.
const RATE_TAU_SECS: f64 = 2.0;
/// Render tick. 10 Hz: comfortably under the 20 Hz convention ceiling,
/// invisible as latency.
const TICK: Duration = Duration::from_millis(100);

fn render_loop(state: &ProgressState, stop: &(Mutex<bool>, Condvar), caps: TermCaps, label: &str) {
    let mut err = std::io::stderr();
    let start = Instant::now();
    let mut first = true;
    let mut hi_ratio = 0.0f64;
    let mut rate = 0.0f64;
    let mut prev_bytes = 0u64;
    let mut prev_t = start;
    let mut last_osc: Option<(u8, Option<u8>)> = None;
    let mut last_plain = Instant::now();
    let mut last_plain_decile = 0u64;
    let spinner_frames = ['|', '/', '-', '\\'];
    let mut tick_n = 0usize;

    if caps.mode == RenderMode::Ansi {
        let _ = err.write_all(b"\x1b[?25l");
    }

    loop {
        let stopped = {
            let (lock, cvar) = stop;
            let mut guard = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // A notify that lands before the wait is not queued (std
            // Condvar), so a stop signalled before this thread reaches
            // its first tick would otherwise cost a full extra tick.
            if !*guard {
                guard = cvar
                    .wait_timeout(guard, TICK)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
            *guard
        };
        let snap = snapshot(state);
        let now = Instant::now();

        // Time-based EWMA over the byte deltas the ticks observe.
        let dt = now.duration_since(prev_t).as_secs_f64().max(1e-3);
        let inst = (snap.done_bytes.saturating_sub(prev_bytes)) as f64 / dt;
        let alpha = (dt / RATE_TAU_SECS).min(1.0);
        rate = alpha * inst + (1.0 - alpha) * rate;
        prev_bytes = snap.done_bytes;
        prev_t = now;

        // Monotonic ratio: totals are a ceiling, never render backwards.
        let ratio = if snap.total_bytes > 0 {
            (snap.done_bytes as f64 / snap.total_bytes as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        hi_ratio = hi_ratio.max(ratio);

        // ETA: suppressed for the first second and near-zero rates.
        let eta = if start.elapsed().as_secs() >= 1 && rate > 1024.0 && snap.total_bytes > 0 {
            Some((snap.total_bytes.saturating_sub(snap.done_bytes)) as f64 / rate)
                .map(|s| s.ceil() as u64)
        } else {
            None
        };

        match caps.mode {
            RenderMode::Ansi => {
                let frame = compose_frame(
                    &snap,
                    &FrameCtx {
                        label,
                        width: caps.width,
                        color: caps.color,
                        first,
                        ratio: hi_ratio,
                        rate,
                        eta,
                        spinner: spinner_frames[tick_n % spinner_frames.len()],
                        done: false,
                    },
                );
                let _ = err.write_all(frame.as_bytes());
                // Terminal-native progress: integer percent, on change.
                let osc = if snap.phase == Phase::Staging && snap.total_bytes > 0 {
                    (1u8, Some((hi_ratio * 100.0).floor() as u8))
                } else {
                    (3u8, None)
                };
                if last_osc != Some(osc) {
                    let _ = err.write_all(osc_progress(osc.0, osc.1).as_bytes());
                    last_osc = Some(osc);
                }
                let _ = err.flush();
                first = false;
            }
            RenderMode::Plain | RenderMode::Off => {
                // Plain threshold lines: every 10% or every 10 s.
                let decile = (hi_ratio * 10.0).floor() as u64;
                if caps.mode == RenderMode::Plain
                    && snap.total_bytes > 0
                    && (decile > last_plain_decile
                        || now.duration_since(last_plain) >= Duration::from_secs(10))
                {
                    let _ = writeln!(
                        err,
                        "[install] {}  {:>3}%  {} / {}  ({}/{} files)",
                        phase_label(snap.phase),
                        (hi_ratio * 100.0).floor() as u32,
                        fmt_bytes(snap.done_bytes.min(snap.total_bytes)),
                        fmt_bytes(snap.total_bytes),
                        snap.done_files,
                        snap.total_files,
                    );
                    last_plain = now;
                    last_plain_decile = decile;
                }
            }
        }

        tick_n += 1;
        if stopped {
            break;
        }
    }

    // Final frame so a finished bar reads 100%, then leave the lines.
    // The phase is `Committing` by now; `done` is what turns line 2
    // back into the full bar.
    if caps.mode == RenderMode::Ansi && state.finished.load(Ordering::Relaxed) {
        let snap = snapshot(state);
        let frame = compose_frame(
            &snap,
            &FrameCtx {
                label,
                width: caps.width,
                color: caps.color,
                first,
                ratio: 1.0,
                rate,
                eta: None,
                spinner: '=',
                done: true,
            },
        );
        let _ = err.write_all(frame.as_bytes());
        let _ = err.flush();
    }
}

#[cfg(test)]
#[path = "tests/progress_render_tests.rs"]
mod tests;
