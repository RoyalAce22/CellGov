//! The console report a finished `boot run` prints.
//!
//! Each block has a formatter that returns lines and a printer that
//! picks the stream. Only the printers touch a console.

use std::time::{Duration, Instant};

use cellgov_boot::prepare::StartupTimings;
use cellgov_boot::step_loop::{compute_untracked, pct, StepTiming};
use cellgov_core::Runtime;

/// Counters the runtime kept that the run reports as anomalies.
#[derive(Debug, Default)]
pub(super) struct RunCounters {
    /// Reads answered zero from a reserved RSX or SPU region.
    pub provisional_reads: u64,
    /// Pending wake responses overwritten before the guest drained them.
    pub response_displacements: usize,
    /// `sys_tty_write` calls whose buffer left mapped memory.
    pub tty_oob_dropped: usize,
    /// `sys_tty_write` calls whose fd did not fit in `u32`.
    pub tty_bogus_fd: usize,
}

impl RunCounters {
    /// A displaced response is the one counter that makes the run's own
    /// result suspect; the others name work the run dropped.
    pub(super) fn had_critical_anomaly(&self) -> bool {
        self.response_displacements > 0
    }
}

/// `part` as a percentage of `total`; a zero total reads as 0.
fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * part as f64 / total as f64
    }
}

/// What each startup stage cost, as reported lines.
pub(super) fn startup_timing_lines(t: &StartupTimings) -> Vec<String> {
    vec![
        "startup timing:".to_string(),
        format!("  file read + mem alloc: {:?}", t.mem_alloc),
        format!("  ELF load:             {:?}", t.elf_load),
        format!("  HLE bind:             {:?}", t.hle_bind),
        format!("  PRX load + resolve:   {:?}", t.prx_load),
        format!("  total startup:        {:?}", t.total()),
        String::new(),
    ]
}

/// One line per counter that moved; a quiet run reports none.
pub(super) fn anomaly_lines(c: &RunCounters) -> Vec<String> {
    let mut out = Vec::new();
    if c.provisional_reads > 0 {
        out.push(format!(
            "provisional_reads: {} (reserved RSX/SPU regions returned zero)",
            c.provisional_reads
        ));
    }
    if c.response_displacements > 0 {
        out.push(format!(
            "syscall_response_displacements: {} (pending wake responses overwritten before drain; lost r3 + out-pointer writes)",
            c.response_displacements
        ));
    }
    if c.tty_oob_dropped > 0 {
        out.push(format!(
            "tty_oob_captures_dropped: {} (sys_tty_write calls whose buffer overflowed guest memory)",
            c.tty_oob_dropped
        ));
    }
    if c.tty_bogus_fd > 0 {
        out.push(format!(
            "tty_bogus_fd_calls: {} (sys_tty_write calls with fd values not fitting in u32)",
            c.tty_bogus_fd
        ));
    }
    out
}

/// Where the step loop's wall time went.
///
/// A loop shorter than the host clock's resolution measures zero, and
/// the block then marks every percentage meaningless.
pub(super) fn step_profile_lines(t: &StepTiming, t_loop: Duration, steps: usize) -> Vec<String> {
    let mut out = vec![String::new(), "profile:".to_string()];
    if t_loop.is_zero() {
        out.push(
            "  WARN: t_loop is zero (clock resolution artifact or instantaneous loop); percentages below are meaningless".to_string(),
        );
    }
    out.push(format!("  total loop:    {t_loop:?}"));
    out.push(format!(
        "  step (sched):  {:?}  ({:.1}%)",
        t.step_time,
        pct(t.step_time, t_loop)
    ));
    out.push(format!(
        "  commit:        {:?}  ({:.1}%)",
        t.commit_time,
        pct(t.commit_time, t_loop)
    ));
    out.push(format!(
        "  coverage tally:{:?}  ({:.1}%)",
        t.coverage_time,
        pct(t.coverage_time, t_loop)
    ));
    match compute_untracked(t_loop, t.step_time, t.commit_time, t.coverage_time) {
        Ok(overhead) => out.push(format!(
            "  other overhead:{:?}  ({:.1}%)",
            overhead,
            pct(overhead, t_loop)
        )),
        Err(excess) => out.push(format!(
            "  other overhead: WARN tracked buckets exceed loop total by {excess:?}"
        )),
    }
    if t_loop.is_zero() {
        out.push("  steps/sec:     n/a (loop time below clock resolution)".to_string());
    } else {
        out.push(format!(
            "  steps/sec:     {:.0}",
            steps as f64 / t_loop.as_secs_f64()
        ));
    }
    out
}

/// One adjacent-instruction pair, as its tally row names it.
pub(super) struct InsnPair(pub &'static str, pub &'static str);

impl std::fmt::Display for InsnPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ; {}", self.0, self.1)
    }
}

/// The top `limit` rows of one frequency tally, headed by its total.
///
/// The total counts every row, including the rows past `limit`.
pub(super) fn frequency_block<T: std::fmt::Display>(
    heading: &str,
    rows: &[(T, u64)],
    limit: usize,
) -> Vec<String> {
    let total: u64 = rows.iter().map(|(_, c)| c).sum();
    let mut out = vec![
        String::new(),
        format!("--- {heading} (raw decoded, top {limit}, total={total}) ---"),
    ];
    for (name, count) in rows.iter().take(limit) {
        out.push(format!(
            "  {:>12}  {:.2}%  {}",
            count,
            percent(*count, total),
            name
        ));
    }
    out
}

/// Rows the per-unit instruction and adjacent-pair blocks report.
pub(super) const FREQUENCY_ROWS: usize = 40;

/// Wall-clock spans of one `boot run`, reported under
/// `CELLGOV_RUNGAME_PROFILE`.
///
/// These spans are host time for display only. Nothing here reaches a
/// scheduling decision or a state hash.
pub(super) struct RunSpans {
    enabled: bool,
    start: Instant,
    prepared: Instant,
    stepped: Instant,
}

impl RunSpans {
    /// Reads the profile toggle once for all run spans.
    ///
    /// # Errors
    ///
    /// Returns an error if the environment value is not a Boolean.
    pub(super) fn start() -> Result<Self, crate::cli::exit::CommandError> {
        let now = Instant::now();
        Ok(Self {
            enabled: crate::cli::env::parse_env_bool(crate::env_vars::RUNGAME_PROFILE)?,
            start: now,
            prepared: now,
            stepped: now,
        })
    }

    /// Close the prepare span.
    pub(super) fn mark_prepared(&mut self) {
        self.prepared = Instant::now();
    }

    /// Close the step-loop span.
    pub(super) fn mark_stepped(&mut self) {
        self.stepped = Instant::now();
    }

    /// The span line, or `None` when the toggle is off.
    pub(super) fn line(&self, steps: usize, dirty_pages_at_steploop_exit: u64) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let end = Instant::now();
        let ms = |a: Instant, b: Instant| b.duration_since(a).as_secs_f64() * 1000.0;
        Some(format!(
            "rungame_profile: prepare={:.2}ms steploop={:.2}ms save={:.2}ms total={:.2}ms \
             steps={steps} dirty_pages_at_steploop_exit={dirty_pages_at_steploop_exit}",
            ms(self.start, self.prepared),
            ms(self.prepared, self.stepped),
            ms(self.stepped, end),
            ms(self.start, end),
        ))
    }
}

/// Read the counters the runtime kept during the loop.
pub(super) fn read_counters(
    rt: &Runtime,
    tty_oob_dropped: usize,
    tty_bogus_fd: usize,
) -> RunCounters {
    RunCounters {
        provisional_reads: rt.memory().provisional_read_count(),
        response_displacements: rt.syscall_responses().displacement_count(),
        tty_oob_dropped,
        tty_bogus_fd,
    }
}

/// Print `lines` to stdout, one per line.
pub(super) fn print_out(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// Print `lines` to stderr, one per line.
pub(super) fn print_err(lines: &[String]) {
    for line in lines {
        eprintln!("{line}");
    }
}

#[cfg(test)]
#[path = "tests/report_tests.rs"]
mod tests;
