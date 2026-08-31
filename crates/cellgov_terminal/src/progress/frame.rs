//! Pure frame composition: snapshot plus presentation inputs in, one
//! terminal-ready buffer out. Nothing here touches a stream or a clock.

use super::state::Snapshot;
use super::task::{Task, DONE_STATUS};
use crate::caps::Style;

/// ASCII bar fill: `=` done, `>` head, `.` remaining, in `width`
/// interior columns.
pub(crate) fn bar_fill(width: usize, ratio: f64) -> String {
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

/// Integer percent, clamped like [`bar_fill`]'s ratio so the two
/// cannot contradict each other.
pub(crate) fn percent(ratio: f64) -> u32 {
    (ratio.clamp(0.0, 1.0) * 100.0).floor() as u32
}

/// `1m12s` / `47s`.
pub(crate) fn fmt_eta(secs: u64) -> String {
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Middle-elide `text` to at most `max` bytes with `...`, keeping the
/// tail (the filename, the title id).
///
/// Item names are ASCII tree paths and identifiers, so the byte
/// budget is a column budget.
pub(crate) fn elide(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    if max <= 3 {
        // A full "..." at max 1 or 2 would break the byte budget and
        // push a frame line past the terminal width.
        return "..."[..max].to_string();
    }
    let keep = max - 3;
    let head = keep / 3;
    let tail = keep - head;
    let head_end = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= head)
        .last()
        .unwrap_or(0);
    let tail_start = text
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= text.len() - tail)
        .unwrap_or(text.len());
    format!("{}...{}", &text[..head_end], &text[tail_start..])
}

/// Printable ASCII only: anything else becomes `?`, so byte length
/// equals display width for the column arithmetic.
pub(crate) fn ascii_only(s: &str) -> String {
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

/// Columns [`push_field`] spends on the separator before `part`.
const FIELD_SEP: usize = 2;

/// Columns line 1 spends outside the verb, the label and the status:
/// the space after the verb, and the `  [` / `]` around the status.
const LINE1_FIXED: usize = 5;

fn push_field(line: &mut String, part: &str) {
    if !line.is_empty() {
        line.push_str("  ");
    }
    line.push_str(part);
}

/// Presentation inputs for one frame, alongside the [`Snapshot`].
pub(crate) struct FrameCtx<'a> {
    pub(crate) task: &'a Task,
    pub(crate) label: &'a str,
    pub(crate) width: usize,
    pub(crate) color: bool,
    /// First frame allocates its lines; later frames cursor-up over them.
    pub(crate) first: bool,
    /// Monotonic high-water fill ratio.
    pub(crate) ratio: f64,
    /// Smoothed amount per second, in the task's unit.
    pub(crate) rate: f64,
    pub(crate) eta: Option<u64>,
    pub(crate) spinner: char,
    /// The task completed, so the denominator applies whatever the
    /// phase.
    pub(crate) done: bool,
}

impl FrameCtx<'_> {
    /// Whether the denominator applies right now, so line 2 can draw a
    /// bar instead of a spinner.
    fn measured(&self, snap: &Snapshot) -> bool {
        snap.total_amount > 0 && (snap.phase == self.task.measured || self.done)
    }
}

/// Compose one 3-line ANSI frame into a single buffer, wrapped in DEC
/// private mode 2026 (synchronized update; terminals that lack it
/// ignore the unknown mode).
///
/// Every visible line is at most `width` columns. A line that wraps
/// occupies two terminal rows, and the next frame's cursor-up by
/// three then lands one row low, leaving a stale row behind on every
/// tick; the label and item-name budgets exist for that reason.
pub(crate) fn compose_frame(snap: &Snapshot, ctx: &FrameCtx<'_>) -> String {
    let task = ctx.task;
    let width = ctx.width;
    let st = Style { on: ctx.color };
    let mut out = String::with_capacity(256);
    out.push_str("\x1b[?2026h");
    if !ctx.first {
        out.push_str("\x1b[3A");
    }

    // Line 1: verb + label + phase. It carries SGR sequences, so it
    // cannot be clipped after the fact the way lines 2 and 3 are --
    // the cut would land inside an escape. Every part is budgeted
    // instead, the label taking whatever the longest status leaves.
    let status = if ctx.done {
        DONE_STATUS
    } else {
        task.phase_label(snap.phase)
    };
    let verb = elide(task.verb, width.saturating_sub(LINE1_FIXED));
    let status_room = width.saturating_sub(verb.len() + LINE1_FIXED);
    let status = elide(status, status_room);
    let label_room =
        width.saturating_sub(verb.len() + LINE1_FIXED + task.max_status_len().min(status_room));
    let label = elide(&ascii_only(ctx.label), label_room);
    out.push_str("\x1b[2K");
    out.push_str(&format!(
        "{}{verb} {label}{}  {}[{status}]{}\n",
        st.bold(),
        st.reset(),
        st.dim(),
        st.reset(),
    ));

    // Line 2: bar or spinner.
    out.push_str("\x1b[2K");
    if ctx.measured(snap) {
        let stats = format!(
            " {:>3}%  {} / {}",
            percent(ctx.ratio),
            task.unit.amount(snap.done_amount.min(snap.total_amount)),
            task.unit.amount(snap.total_amount),
        );
        let bar_w = width.saturating_sub(stats.len() + 2).max(10);
        let mut line = format!("[{}]{stats}", bar_fill(bar_w, ctx.ratio));
        clip_columns(&mut line, width);
        out.push_str(&line);
        out.push('\n');
    } else if ctx.done {
        out.push_str("= done\n");
    } else {
        let mut line = format!("{} {}...", ctx.spinner, task.phase_label(snap.phase));
        clip_columns(&mut line, width);
        out.push_str(&line);
        out.push('\n');
    }

    // Line 3: item counter, rate, ETA, current item.
    out.push_str("\x1b[2K");
    let mut line = String::new();
    if !task.items.is_empty() {
        push_field(
            &mut line,
            &format!("{}/{} {}", snap.done_items, snap.total_items, task.items),
        );
    }
    if snap.phase == task.measured && ctx.rate > 1.0 {
        push_field(&mut line, &task.unit.rate(ctx.rate));
        if let Some(e) = ctx.eta {
            push_field(&mut line, &format!("ETA {}", fmt_eta(e)));
        }
    }
    // The item name joins only when it and its separator fit; at width
    // 40 with a rate and an ETA, neither does.
    let sep = if line.is_empty() { 0 } else { FIELD_SEP };
    let room = width.saturating_sub(line.len() + sep);
    if room > 0 && !snap.current.is_empty() {
        push_field(&mut line, &elide(&ascii_only(&snap.current), room));
    }
    clip_columns(&mut line, width);
    out.push_str(&format!("{}{line}{}\n", st.dim(), st.reset()));

    out.push_str("\x1b[?2026l");
    out
}

/// One plain-mode threshold line.
pub(crate) fn plain_line(snap: &Snapshot, task: &Task, ratio: f64) -> String {
    let mut line = format!(
        "[{}] {}  {:>3}%  {} / {}",
        task.tag,
        task.phase_label(snap.phase),
        percent(ratio),
        task.unit.amount(snap.done_amount.min(snap.total_amount)),
        task.unit.amount(snap.total_amount),
    );
    if !task.items.is_empty() {
        line.push_str(&format!(
            "  ({}/{} {})",
            snap.done_items, snap.total_items, task.items
        ));
    }
    line
}

/// OSC 9;4 terminal-native progress (taskbar / dock).
pub(crate) fn osc_progress(state: u8, pct: Option<u8>) -> String {
    match pct {
        Some(p) => format!("\x1b]9;4;{state};{}\x07", p.min(100)),
        None => format!("\x1b]9;4;{state}\x07"),
    }
}

#[cfg(test)]
#[path = "tests/frame_tests.rs"]
mod tests;
