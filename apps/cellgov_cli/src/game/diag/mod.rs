//! Diagnostic formatting for `run-game`: reads runtime state, produces strings.
//!
//! `pc_ring` readers assume a single-threaded stepper; a concurrent writer
//! would tear reads.

mod exit;
mod fault;
mod helpers;
mod rings;
mod summary;
mod trace;

pub(super) use exit::{format_max_steps, format_process_exit, ProcessExitInfo, TtyCapture};
pub(super) use fault::{format_commit_fault, format_deadlock, format_fault};
pub(super) use helpers::{
    ascii_safe_preview, fetch_raw_at, format_hle_idx, longest_readable_prefix, region_label_at,
};
pub(super) use rings::{append_orphan_exit_info, append_syscall_ring};
pub(super) use summary::{
    print_hle_summary, print_insn_coverage, print_shadow_stats, print_top_pcs,
};
pub(super) use trace::print_trace_line;
