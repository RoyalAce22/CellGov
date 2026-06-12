//! [`StepLoopCtx`] -- mutable per-loop state plumbed through
//! [`super::step_loop`].

use std::time::Instant;

use crate::game::diag::{ProcessExitInfo, TtyCapture};
use crate::game::manifest;
use crate::game::step_loop::ring::{RingCursor, PC_RING_SIZE, SYSCALL_RING_SIZE};
use crate::game::step_loop::timing::StepTiming;

pub(in crate::game) struct StepLoopCtx<'a> {
    pub(in crate::game) steps: &'a mut usize,
    pub(in crate::game) distinct_pcs: &'a mut std::collections::BTreeSet<u64>,
    pub(in crate::game) hle_calls: &'a mut std::collections::BTreeMap<u32, usize>,
    pub(in crate::game) insn_coverage: &'a mut std::collections::BTreeMap<&'static str, usize>,
    pub(in crate::game) trace: bool,
    pub(in crate::game) timing: &'a mut Option<StepTiming>,
    pub(in crate::game) loop_start: Instant,
    pub(in crate::game) pc_ring: [u64; PC_RING_SIZE],
    pub(in crate::game) pc_ring_cursor: RingCursor,
    pub(in crate::game) last_tty: Option<TtyCapture>,
    pub(in crate::game) last_exit: Option<ProcessExitInfo>,
    pub(in crate::game) syscall_ring: [(u64, u64); SYSCALL_RING_SIZE],
    pub(in crate::game) syscall_ring_cursor: RingCursor,
    /// Top entries identify busy-loop bodies on max-steps.
    pub(in crate::game) pc_hits: &'a mut std::collections::BTreeMap<u64, u64>,
    pub(in crate::game) checkpoint: manifest::CheckpointTrigger,
    /// `sys_tty_write` calls dropped because `buf + len` exceeded mapped memory.
    pub(in crate::game) tty_oob_count: usize,
    /// `sys_tty_write` calls whose fd exceeded `u32::MAX` (narrowed to sentinel).
    pub(in crate::game) bogus_fd_count: usize,
    /// Address+length pairs to hex-dump from guest memory at fault
    /// time. Empty by default; set via `run-game --dump-mem-fault`.
    pub(in crate::game) dump_mem_fault_ranges: &'a [(u64, u64)],
}
