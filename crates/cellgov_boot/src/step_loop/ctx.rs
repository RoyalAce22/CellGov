//! [`StepLoopCtx`] -- mutable per-loop state plumbed through
//! [`super::step_loop`].

use std::rc::Rc;
use std::time::Instant;

use crate::diag::{ProcessExitInfo, TtyCapture};
use crate::manifest;
use crate::step_loop::ring::{PcRing, SyscallRing};
use crate::step_loop::timing::StepTiming;
use crate::{BootSink, ChildInitPlans};

/// What the diagnostic step loop reads and accumulates.
pub struct StepLoopCtx<'a> {
    /// Steps retired; the loop requires a zero here at entry.
    pub steps: &'a mut usize,
    /// HLE import index -> call count.
    pub hle_calls: &'a mut std::collections::BTreeMap<u32, usize>,
    /// Instruction name -> retire count.
    pub insn_coverage: &'a mut std::collections::BTreeMap<&'static str, usize>,
    /// Whether the loop reports each step as it retires.
    pub trace: bool,
    /// Per-bucket wall time, when the caller asked to profile.
    pub timing: &'a mut Option<StepTiming>,
    /// When the loop started, for the untracked-time residue.
    pub loop_start: Instant,
    /// Last PCs attempted.
    pub pc_ring: PcRing,
    /// Last guest TTY write captured.
    pub last_tty: Option<TtyCapture>,
    /// Last process-exit syscall seen.
    pub last_exit: Option<ProcessExitInfo>,
    /// Last syscalls dispatched.
    pub syscall_ring: SyscallRing,
    /// Top entries identify busy-loop bodies on max-steps.
    pub pc_hits: &'a mut std::collections::BTreeMap<(cellgov_core::AddressSpaceId, u64), u64>,
    /// Where the run should stop.
    pub checkpoint: manifest::CheckpointTrigger,
    /// `sys_tty_write` calls dropped because `buf + len` exceeded mapped memory.
    pub tty_oob_count: usize,
    /// `sys_tty_write` calls whose fd exceeded `u32::MAX` (narrowed to sentinel).
    pub bogus_fd_count: usize,
    /// Address+length pairs to hex-dump from guest memory at fault
    /// time. Empty by default; set via `boot run --dump-mem-fault`.
    pub dump_mem_fault_ranges: &'a [(u64, u64)],
    /// Wipe the host's observability after every committed step
    /// (`CELLGOV_OBS_NULL_SINK=1`). The inertness gate: a boot run
    /// this way must produce byte-identical state traces.
    pub obs_null_sink: bool,
    /// Init plans the spawn loader staged for children parked behind
    /// their module_start pass.
    pub child_init: &'a ChildInitPlans,
    /// Where the loop reports retired steps.
    pub progress: &'a dyn cellgov_terminal::progress::ProgressSink,
    /// Where the loop writes its narration.
    pub sink: Rc<dyn BootSink>,
}
