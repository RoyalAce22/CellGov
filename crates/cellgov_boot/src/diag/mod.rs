//! Diagnostic formatting for a boot: reads runtime state, produces
//! strings.
//!
//! `pc_ring` readers assume a single-threaded stepper; a concurrent
//! writer would tear reads.

mod exit;
mod fault;
mod helpers;
mod rings;
mod summary;
mod trace;

pub(crate) use fault::{format_commit_fault, format_deadlock, format_fault, unit_memory};
pub(crate) use helpers::{
    ascii_safe_preview, fetch_raw_at, format_hle_idx, longest_readable_prefix, region_label_at,
};
pub(crate) use rings::{append_orphan_exit_info, append_pc_ring_with_decode, append_syscall_ring};
pub(crate) use trace::report_trace_line;

pub use exit::{format_max_steps, format_process_exit, ProcessExitInfo, TtyCapture};
pub use summary::{report_hle_summary, report_insn_coverage, report_shadow_stats, report_top_pcs};
