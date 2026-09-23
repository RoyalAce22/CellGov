//! The caller's description of the work a bar is reporting on.

/// What the denominator counts, selecting the amount, rate and ETA
/// formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Bytes, formatted in binary multiples (`1.94 GiB`).
    Bytes,
    /// Files, formatted as a count.
    Files,
    /// Interpreter steps, formatted as a count.
    Steps,
    /// Fuzz cases, formatted as a count.
    Cases,
    /// Anything else countable, formatted as a count.
    Items,
}

impl Unit {
    /// Plural noun for a rate, empty when the amount format already
    /// names the unit.
    const fn noun(self) -> &'static str {
        match self {
            Self::Bytes => "",
            Self::Files => "files",
            Self::Steps => "steps",
            Self::Cases => "cases",
            Self::Items => "items",
        }
    }

    /// `n` in this unit: `1.94 GiB`, or `12.4M` for a count.
    pub(crate) fn amount(self, n: u64) -> String {
        match self {
            Self::Bytes => fmt_bytes(n),
            _ => fmt_count(n),
        }
    }

    /// `n` with its noun: `43.0k steps`, or `512 KiB`, whose format
    /// already names the unit.
    pub(crate) fn tally(self, n: u64) -> String {
        match self {
            Self::Bytes => fmt_bytes(n),
            _ => format!("{} {}", fmt_count(n), self.noun()),
        }
    }

    /// `per_sec` in this unit: `38.2 MiB/s`, or `12.4M steps/s`.
    pub(crate) fn rate(self, per_sec: f64) -> String {
        let n = per_sec.max(0.0) as u64;
        match self {
            Self::Bytes => format!("{}/s", fmt_bytes(n)),
            _ => format!("{} {}/s", fmt_count(n), self.noun()),
        }
    }

    /// Rate below which an ETA is noise rather than a prediction.
    ///
    /// A byte rate under a kibibyte per second predicts nothing; one
    /// step or file per second is a real measurement.
    pub(crate) const fn eta_rate_floor(self) -> f64 {
        match self {
            Self::Bytes => 1024.0,
            _ => 1.0,
        }
    }
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

/// `12.4M` / `1.2k` / `947`.
///
/// Like [`fmt_bytes`], the column just below a multiple rounds up into
/// an extra digit: `999_999` formats as `1000.0k`.
fn fmt_count(n: u64) -> String {
    const G: f64 = 1e9;
    const M: f64 = 1e6;
    const K: f64 = 1e3;
    let x = n as f64;
    if x >= G {
        format!("{:.2}G", x / G)
    } else if x >= M {
        format!("{:.1}M", x / M)
    } else if x >= K {
        format!("{:.1}k", x / K)
    } else {
        format!("{n}")
    }
}

/// What a bar is reporting on, supplied once by the caller.
///
/// Every string here reaches the terminal unsanitized, so all of them
/// must be ASCII.
#[derive(Debug, Clone, Copy)]
pub struct Task {
    /// Line 1's verb: `Installing`, `Downloading`, `Booting`.
    pub verb: &'static str,
    /// Bracketed prefix on plain-mode threshold lines, without the
    /// brackets: `install` renders as `[install]`.
    pub tag: &'static str,
    /// Phase labels, indexed by the sink's phase code.
    pub phases: &'static [&'static str],
    /// Index into [`Self::phases`] of the one phase the denominator
    /// measures; every other phase renders as an indeterminate spinner.
    pub measured: u8,
    /// What the denominator counts.
    pub unit: Unit,
    /// Plural noun for the secondary item counter (`files`), or empty
    /// when the task has no item counter to show.
    pub items: &'static str,
    /// The command writes its own lines to the terminal while working,
    /// so the bar must not use in-place frames.
    pub streaming: bool,
}

impl Task {
    /// Label for phase code `code`.
    pub(crate) fn phase_label(&self, code: u8) -> &'static str {
        // A code past a populated table means the caller's phase enum
        // and its label table have drifted; an empty table is not
        // drift and answers `FALLBACK_STATUS`.
        debug_assert!(
            self.phases.is_empty() || (code as usize) < self.phases.len(),
            "phase code {code} is past the {} label(s) of the {} task",
            self.phases.len(),
            self.verb
        );
        self.phases
            .get(code as usize)
            .copied()
            .or_else(|| self.phases.first().copied())
            .unwrap_or(FALLBACK_STATUS)
    }

    /// Longest status text line 1 can show, so its label budget does
    /// not change as phases advance.
    pub(crate) fn max_status_len(&self) -> usize {
        // `phase_label(0)` is the empty-table fallback, which is longer
        // than `DONE_STATUS`.
        self.phases
            .iter()
            .map(|l| l.len())
            .chain([DONE_STATUS.len(), self.phase_label(0).len()])
            .max()
            .unwrap_or(0)
    }
}

/// Line 1's status once the task has completed.
pub(crate) const DONE_STATUS: &str = "done";

/// Line 1's status when a task's phase table cannot answer.
pub(crate) const FALLBACK_STATUS: &str = "working";

#[cfg(test)]
#[path = "tests/task_tests.rs"]
mod tests;
