//! The live display: a render thread that owns stderr, and the
//! teardown paths that leave the screen sane however the run ends.

use super::frame::{
    compose_frame, counting, osc_progress, plain_counting_line, plain_indeterminate_line,
    plain_line, FrameCtx,
};
use super::state::ProgressState;
use super::task::Task;
use crate::caps::{RenderMode, TermCaps};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// EWMA time constant for the rate, in seconds.
const RATE_TAU_SECS: f64 = 2.0;
/// Render tick. 10 Hz: comfortably under the 20 Hz convention ceiling,
/// invisible as latency.
const TICK: Duration = Duration::from_millis(100);
/// How long the panic hook waits for the render thread to stop before
/// restoring the terminal anyway. Long enough for a thread mid-tick,
/// short enough not to stall a crash.
const PANIC_DRAIN: Duration = Duration::from_millis(250);

/// Hides the cursor for the in-place frame; every restore answers it.
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
/// The restore the non-unwinding paths write: cursor back, taskbar
/// state cleared, and a newline off the frame's last line.
const RESTORE: &[u8] = b"\x1b[?25h\x1b]9;4;0\x07\n";

/// Stop/exit handshake between a bar and its render thread.
#[derive(Debug, Default)]
struct StopFlags {
    /// The bar asked the render thread to stop.
    stop: bool,
    /// The render thread has left its loop and written its last byte.
    exited: bool,
}

/// The [`StopFlags`] handshake, shared with the panic hook's registry.
#[derive(Debug, Default)]
struct BarStop {
    flags: Mutex<StopFlags>,
    cv: Condvar,
}

impl BarStop {
    fn lock(&self) -> std::sync::MutexGuard<'_, StopFlags> {
        self.flags
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn request_stop(&self) {
        self.lock().stop = true;
        self.cv.notify_all();
    }

    /// Sleep one tick, or until a stop is requested. Reports whether
    /// this is the last tick.
    fn wait_tick(&self) -> bool {
        let mut guard = self.lock();
        // A notify that lands before the wait is not queued (std
        // Condvar), so a stop signalled before this thread reaches its
        // first tick would otherwise cost a full extra tick.
        if !guard.stop {
            guard = self
                .cv
                .wait_timeout(guard, TICK)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        guard.stop
    }

    fn mark_exited(&self) {
        self.lock().exited = true;
        self.cv.notify_all();
    }

    /// Wait up to `timeout` for the render thread to leave its loop.
    ///
    /// Runs inside the panic hook, where a second panic aborts the
    /// process, so the deadline is elapsed-time arithmetic: `Instant +
    /// Duration` panics on an unrepresentable sum.
    fn wait_for_exit(&self, timeout: Duration) {
        let mut guard = self.lock();
        let start = Instant::now();
        while !guard.exited {
            let left = timeout.saturating_sub(start.elapsed());
            if left.is_zero() {
                break;
            }
            guard = self
                .cv
                .wait_timeout(guard, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}

/// The terminal owes a restore: an `Ansi` bar hid the cursor and set a
/// taskbar state.
///
/// Whoever swaps it false claims the restore. The panic hook and the
/// [`Drop`] that follows it both run on the way out of a panicking
/// command, and the Ctrl-C handler runs beside whatever the main
/// thread is doing; only one of them may write.
static BAR_ACTIVE: AtomicBool = AtomicBool::new(false);

/// The live bar's handshake, for the panic hook to reach.
///
/// One live bar per process, so one slot. Nothing that can panic runs
/// while this lock is held -- a panic there would deadlock the hook
/// against itself.
static LIVE_BAR: Mutex<Option<Arc<BarStop>>> = Mutex::new(None);

fn live_bar_slot() -> std::sync::MutexGuard<'static, Option<Arc<BarStop>>> {
    LIVE_BAR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Install the cursor/OSC-restoring panic hook once, chained in front
/// of the previous hook so the panic message lands on a sane screen.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            quiesce_for_panic();
            prev(info);
        }));
    });
}

/// Install the Ctrl-C handler once; it runs
/// [`crate::interrupt::exit_interrupted`].
///
/// The handler runs on the signal crate's own thread while the render
/// thread may be mid-tick. [`quiesce_for_panic`] bounds the wait for
/// that thread, and the stderr handle locks per write, so a tick in
/// flight delays the restore by one write. The hook reports a handler
/// the OS refuses once, before the render thread starts; the bar then
/// owes its restore to the unwinding paths alone.
fn install_interrupt_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let installed = ctrlc::set_handler(|| crate::interrupt::exit_interrupted());
        if let Err(e) = installed {
            let mut err = std::io::stderr();
            let _ = writeln!(
                err,
                "cellgov: no Ctrl-C handler installed ({e}); an interrupted bar \
                 leaves the cursor hidden"
            );
        }
    });
}

/// Stop the live bar and restore the terminal.
///
/// This serves the exit paths that do not unwind -- `process::exit`
/// after a refusal, and the Ctrl-C handler -- where [`ProgressBar`]'s
/// [`Drop`] never runs and the cursor stays hidden for the shell that
/// follows.
///
/// The call is safe with no bar running, and safe to repeat. It
/// deregisters the bar without joining it, so the owning
/// [`ProgressBar`] still completes its own teardown.
pub fn release_terminal() {
    quiesce_for_panic();
}

/// Stop the render thread and restore the terminal, before the
/// previous hook prints.
///
/// The hook waits for the thread to leave its loop, so a tick in
/// flight cannot cursor-up over the panic message. The wait is
/// bounded: the panicking thread may be the render thread itself.
fn quiesce_for_panic() {
    let live = live_bar_slot().take();
    let stalled = live.is_some_and(|stop| {
        stop.request_stop();
        stop.wait_for_exit(PANIC_DRAIN);
        !stop.lock().exited
    });
    let mut err = std::io::stderr();
    if BAR_ACTIVE.swap(false, Ordering::Relaxed) {
        let _ = err.write_all(RESTORE);
    }
    // A drain that spent its whole budget leaves a render thread free
    // to cursor-up over whatever prints next.
    if stalled {
        let _ = writeln!(
            err,
            "cellgov: the progress render thread did not stop within {} ms; \
             the output below may be overwritten",
            PANIC_DRAIN.as_millis(),
        );
    }
    let _ = err.flush();
}

/// The live progress display: owns the render thread and stderr while
/// running.
///
/// A Ctrl-C runs [`release_terminal`] on the handler's thread, then
/// ends the process as the default action would. The shell that
/// follows gets its cursor back, and a loop that drives the command
/// sees the interrupt. A kill that runs no handler, or a crash past
/// the panic hook, still leaves the cursor hidden.
pub struct ProgressBar {
    state: Arc<ProgressState>,
    stop: Arc<BarStop>,
    handle: Option<std::thread::JoinHandle<()>>,
    caps: TermCaps,
    /// [`Self::finish`] or [`Self::abort`] already ran, so [`Drop`]
    /// has nothing left to do.
    torn_down: bool,
}

impl ProgressBar {
    /// Start rendering `task` for `label` (a no-op shell in
    /// [`RenderMode::Off`]).
    ///
    /// A [`Task::streaming`] task caps the mode at [`RenderMode::Plain`]:
    /// its own lines would scroll the terminal out from under the
    /// in-place frame's cursor arithmetic.
    pub fn start(caps: TermCaps, task: &'static Task, label: &str) -> Self {
        let caps = if task.streaming {
            caps.capped_at_plain()
        } else {
            caps
        };
        let state = Arc::new(ProgressState::new());
        let stop = Arc::new(BarStop::default());
        let handle = match caps.mode {
            RenderMode::Off => None,
            mode => {
                install_panic_hook();
                if mode == RenderMode::Ansi {
                    install_interrupt_hook();
                    BAR_ACTIVE.store(true, Ordering::Relaxed);
                    // The hide goes out on the thread that claimed the
                    // restore, so a restore that finds the claim finds
                    // the hide ahead of it on the stream, whether or
                    // not the render thread has started.
                    let mut err = std::io::stderr();
                    let _ = err.write_all(HIDE_CURSOR);
                    let _ = err.flush();
                }
                // The assert reads the slot without claiming it, so a
                // debug build's panic leaves the live bar registered
                // and joinable instead of a handshake no thread
                // answers. The slot lock is released before the
                // assert: the panic hook takes that same lock.
                debug_assert!(
                    live_bar_slot().is_none(),
                    "one live bar per process: a second bar leaves the first's \
                     render thread unreachable from the panic hook"
                );
                // Registered before the spawn, so a panic between the
                // two still finds a handshake to signal.
                live_bar_slot().replace(Arc::clone(&stop));
                let st = Arc::clone(&state);
                let sp = Arc::clone(&stop);
                let label = label.to_string();
                Some(std::thread::spawn(move || {
                    render_loop(&st, &sp, caps, task, &label);
                }))
            }
        };
        Self {
            state,
            stop,
            handle,
            caps,
            torn_down: false,
        }
    }

    /// The sink to hand to the instrumented code.
    #[must_use]
    pub fn sink(&self) -> Arc<ProgressState> {
        Arc::clone(&self.state)
    }

    fn stop_thread(&mut self) {
        if let Some(h) = self.handle.take() {
            self.stop.request_stop();
            let _ = h.join();
        }
        // Runs whether or not a thread was started, so the slot never
        // outlives the bar that owns it.
        let mut slot = live_bar_slot();
        if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &self.stop)) {
            *slot = None;
        }
    }

    /// The sequence that restores the cursor and clears the
    /// terminal-native progress state, optionally flashing the error
    /// state first -- or `None` when this teardown does not own the
    /// restore (see [`BAR_ACTIVE`]).
    fn restore_sequence(&self, error: bool) -> Option<String> {
        if self.caps.mode != RenderMode::Ansi || !BAR_ACTIVE.swap(false, Ordering::Relaxed) {
            return None;
        }
        let mut out = String::from("\x1b[?25h");
        if error {
            out.push_str(&osc_progress(2, None));
        }
        out.push_str(&osc_progress(0, None));
        Some(out)
    }

    fn restore_terminal(&mut self, error: bool) {
        if let Some(seq) = self.restore_sequence(error) {
            let mut err = std::io::stderr();
            let _ = err.write_all(seq.as_bytes());
            let _ = err.flush();
        }
    }

    /// Graceful teardown after successful work: final frame, cursor
    /// restored, taskbar state cleared.
    pub fn finish(mut self) {
        self.state.finished.store(true, Ordering::Relaxed);
        self.stop_thread();
        self.restore_terminal(false);
        self.torn_down = true;
    }

    /// Teardown after a failure: error state to the taskbar, then
    /// cleared, cursor restored, bar left on screen above the error
    /// message the caller is about to print.
    pub fn abort(mut self) {
        self.stop_thread();
        self.restore_terminal(true);
        self.torn_down = true;
    }
}

impl Drop for ProgressBar {
    /// Insurance for an early return that drops the bar without
    /// [`ProgressBar::finish`] or [`ProgressBar::abort`]: leaving the
    /// render thread alive and the cursor hidden outlives the command.
    fn drop(&mut self) {
        if self.torn_down {
            return;
        }
        self.stop_thread();
        self.restore_terminal(false);
    }
}

/// One tick's inputs to the plain-mode threshold decision.
struct PlainDue {
    /// Zero until the caller declares its denominator.
    total_amount: u64,
    /// The task wrote no plain line yet.
    first: bool,
    /// The render loop breaks after this tick.
    ending: bool,
    /// Tenths of the high-water ratio this tick sees.
    decile: u64,
    /// The decile the last written line reported.
    last_decile: u64,
    /// Time since the last written line.
    since_last: Duration,
}

/// How long a plain-mode task may run without saying anything.
const PLAIN_SILENCE: Duration = Duration::from_secs(10);

/// Whether this tick writes a plain-mode threshold line.
///
/// - A measured task also speaks on its first tick and its last, so
///   one that never crosses a decile still prints.
/// - A phase with no denominator answers the silence budget alone.
fn plain_due(t: &PlainDue) -> bool {
    t.since_last >= PLAIN_SILENCE
        || (t.total_amount > 0 && (t.first || t.ending || t.decile > t.last_decile))
}

/// Seconds until `done` reaches `total` at `rate`, or `None` when there
/// is nothing to predict:
///
/// - no denominator;
/// - a rate at or under the unit's floor;
/// - less than a second elapsed, so no rate exists yet;
/// - a run at or past its finish line.
fn eta_secs(elapsed: Duration, rate: f64, floor: f64, done: u64, total: u64) -> Option<u64> {
    if elapsed.as_secs() < 1 || rate <= floor || total == 0 || done >= total {
        return None;
    }
    Some(((total - done) as f64 / rate).ceil() as u64)
}

fn render_loop(state: &ProgressState, stop: &BarStop, caps: TermCaps, task: &Task, label: &str) {
    let mut err = std::io::stderr();
    let start = Instant::now();
    let mut first = true;
    let mut hi_ratio = 0.0f64;
    let mut rate = 0.0f64;
    let mut prev_advanced = 0u64;
    let mut prev_t = start;
    let mut last_osc: Option<(u8, Option<u8>)> = None;
    let mut last_plain = Instant::now();
    let mut last_plain_decile = 0u64;
    let mut first_plain = true;
    let spinner_frames = ['|', '/', '-', '\\'];
    let mut tick_n = 0usize;

    loop {
        let stopped = stop.wait_tick();
        // Sampled before the snapshot: the tick that draws the closing
        // line must not read counters from before the finish it acts
        // on.
        let ending = stopped || state.finished.load(Ordering::Relaxed);
        let snap = state.snapshot();
        let now = Instant::now();

        // Time-based EWMA over the deltas the ticks observe, measured
        // on what this run moved so a resumed transfer's preset does
        // not register as an opening burst.
        let dt = now.duration_since(prev_t).as_secs_f64().max(1e-3);
        let advanced = snap.advanced();
        let inst = (advanced.saturating_sub(prev_advanced)) as f64 / dt;
        let alpha = (dt / RATE_TAU_SECS).min(1.0);
        rate = alpha * inst + (1.0 - alpha) * rate;
        prev_advanced = advanced;
        prev_t = now;

        // Monotonic ratio: totals are a ceiling, never render backwards.
        let ratio = if snap.total_amount > 0 {
            (snap.done_amount as f64 / snap.total_amount as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        hi_ratio = hi_ratio.max(ratio);

        let elapsed = start.elapsed();
        let eta = eta_secs(
            elapsed,
            rate,
            task.unit.eta_rate_floor(),
            snap.done_amount,
            snap.total_amount,
        );

        match caps.mode {
            RenderMode::Ansi => {
                let frame = compose_frame(
                    &snap,
                    &FrameCtx {
                        task,
                        label,
                        width: caps.width,
                        color: caps.color,
                        first,
                        ratio: hi_ratio,
                        rate,
                        eta,
                        elapsed_secs: elapsed.as_secs(),
                        spinner: spinner_frames[tick_n % spinner_frames.len()],
                        done: false,
                    },
                );
                let _ = err.write_all(frame.as_bytes());
                let osc = if snap.phase == task.measured && snap.total_amount > 0 {
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
                let decile = (hi_ratio * 10.0).floor() as u64;
                if caps.mode == RenderMode::Plain
                    && plain_due(&PlainDue {
                        total_amount: snap.total_amount,
                        first: first_plain,
                        ending,
                        decile,
                        last_decile: last_plain_decile,
                        since_last: now.duration_since(last_plain),
                    })
                {
                    let line = if snap.total_amount > 0 {
                        plain_line(&snap, task, hi_ratio)
                    } else if counting(&snap, task) {
                        plain_counting_line(&snap, task, rate, elapsed.as_secs())
                    } else {
                        plain_indeterminate_line(&snap, task)
                    };
                    let _ = writeln!(err, "{line}");
                    first_plain = false;
                    last_plain = now;
                    last_plain_decile = decile;
                }
            }
        }

        tick_n += 1;
        if ending {
            break;
        }
    }

    // Final frame so a finished bar reads 100%, then leave the lines.
    // The phase is past the measured one by now, so `done` is what
    // keeps line 2 a bar.
    if caps.mode == RenderMode::Ansi && state.finished.load(Ordering::Relaxed) {
        let snap = state.snapshot();
        let frame = compose_frame(
            &snap,
            &FrameCtx {
                task,
                label,
                width: caps.width,
                color: caps.color,
                first,
                ratio: 1.0,
                rate,
                eta: None,
                elapsed_secs: start.elapsed().as_secs(),
                spinner: '=',
                done: true,
            },
        );
        let _ = err.write_all(frame.as_bytes());
        let _ = err.flush();
    }

    stop.mark_exited();
}

#[cfg(test)]
#[path = "tests/bar_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/interrupt_tests.rs"]
mod interrupt_tests;

#[cfg(test)]
#[path = "tests/eta_tests.rs"]
mod eta_tests;
