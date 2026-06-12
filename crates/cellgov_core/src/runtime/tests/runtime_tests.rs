//! Shared fixture units for the runtime suite; the themed sub-suites are declared below.

use super::*;

use cellgov_effects::Effect;

use cellgov_event::UnitId;

use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};

use cellgov_mem::GuestMemory;

use cellgov_time::{Budget, Epoch, GuestTicks, InstructionCost};

use cellgov_trace::TraceWriter;

use std::cell::Cell;

#[path = "runtime_determinism_tests.rs"]
mod determinism;
#[path = "runtime_dma_tests.rs"]
mod dma;
#[path = "runtime_fast_path_tests.rs"]
mod fast_path;
#[path = "runtime_lv2_apply_tests.rs"]
mod lv2_apply;
#[path = "runtime_rsx_tests.rs"]
mod rsx;
#[path = "runtime_state_hash_tests.rs"]
mod state_hashes;
#[path = "runtime_step_tests.rs"]
mod stepping;
#[path = "runtime_trace_tests.rs"]
mod trace_records;
#[path = "runtime_wake_tests.rs"]
mod wakes;

#[derive(Clone)]

struct CountingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
}

impl CountingUnit {
    fn new(id: UnitId, max: u64) -> Self {
        Self {
            id,
            steps: Cell::new(0),
            max,
        }
    }
}

impl ExecutionUnit for CountingUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.max {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        effects.push(Effect::TraceMarker {
            marker: n as u32,
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn build(memory_size: usize, budget: u64, max_steps: usize) -> Runtime {
    Runtime::new(
        GuestMemory::new(memory_size),
        Budget::new(budget),
        max_steps,
    )
}

#[derive(Clone)]

struct WritingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
}

impl ExecutionUnit for WritingUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.max {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        let bytes = vec![n as u8; 4];
        let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
        effects.push(Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(bytes),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

type FullStateTuple = (u64, [u64; 32], u64, u64, u64, u32);

#[derive(Clone)]

struct StateHashEmittingUnit {
    id: UnitId,
    pairs_per_step: Vec<Vec<(u64, u64)>>,
    full_per_step: Vec<Vec<FullStateTuple>>,
    step_idx: Cell<usize>,
}

impl ExecutionUnit for StateHashEmittingUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.step_idx.get() >= self.pairs_per_step.len() {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.step_idx.set(self.step_idx.get() + 1);
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> Self::Snapshot {}
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        let i = self.step_idx.get();
        if i == 0 || i > self.pairs_per_step.len() {
            return vec![];
        }
        self.pairs_per_step[i - 1].clone()
    }
    fn drain_retired_state_full(&mut self) -> Vec<FullStateTuple> {
        let i = self.step_idx.get();
        if i == 0 || i > self.full_per_step.len() {
            return vec![];
        }
        self.full_per_step[i - 1].clone()
    }
}

#[derive(Clone)]

struct SilentUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
}

impl ExecutionUnit for SilentUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.max {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[derive(Clone)]

struct ReservationDriverUnit {
    id: UnitId,
    steps: Cell<u64>,
    line_addr: u64,
}

impl ExecutionUnit for ReservationDriverUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        let n = self.steps.get() + 1;
        self.steps.set(n);
        match n {
            1 => {
                effects.push(Effect::ReservationAcquire {
                    line_addr: self.line_addr,
                    source: self.id,
                });
            }
            2 => {
                let range = ByteRange::new(GuestAddr::new(self.line_addr), 4).unwrap();
                effects.push(Effect::SharedWriteIntent {
                    range,
                    bytes: WritePayload::new(vec![0xAA; 4]),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
            }
            _ => {}
        }
        let yield_reason = if n >= 2 {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[derive(Clone)]

struct RsxFlipCommandEmitterUnit {
    id: UnitId,
    steps: Cell<u64>,
    fifo_base: u32,
    buffer_index: u32,
}

impl ExecutionUnit for RsxFlipCommandEmitterUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use crate::rsx::control_register;
        use crate::rsx::method::{GCM_FLIP_COMMAND, NV_COUNT_SHIFT};
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        self.steps.set(1);
        let header: u32 = (1u32 << NV_COUNT_SHIFT) | (GCM_FLIP_COMMAND as u32);
        let mut fifo_bytes: Vec<u8> = Vec::with_capacity(8);
        fifo_bytes.extend_from_slice(&header.to_be_bytes());
        fifo_bytes.extend_from_slice(&self.buffer_index.to_be_bytes());
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(self.fifo_base as u64), 8).unwrap(),
            bytes: WritePayload::new(fifo_bytes),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(control_register::PUT_ADDR as u64), 4).unwrap(),
            bytes: WritePayload::from_slice(&(self.fifo_base + 8).to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[derive(Clone)]

struct RsxFlipRequestEmitterUnit {
    id: UnitId,
    steps: Cell<u64>,
    buffer_index: u8,
}

impl ExecutionUnit for RsxFlipRequestEmitterUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps.set(1);
        effects.push(Effect::RsxFlipRequest {
            buffer_index: self.buffer_index,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[derive(Clone)]

struct RsxControlWriterUnit {
    id: UnitId,
    steps: Cell<u64>,
    slot_addr: u64,
    value: u32,
}

impl ExecutionUnit for RsxControlWriterUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        self.steps.set(1);
        let range = ByteRange::new(GuestAddr::new(self.slot_addr), 4).unwrap();
        effects.push(Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&self.value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn build_with_rsx_writable() -> Runtime {
    use cellgov_mem::{GuestMemory, PageSize, Region};
    let regions = vec![
        Region::new(0, 0x1000, "flat", PageSize::Page4K),
        Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
    ];
    let mem = GuestMemory::from_regions(regions).expect("regions non-overlapping");
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    // Seed identity iomap covering the flat region; mirrors the
    // build_with_rsx_and_label_region helper. Required after
    // IoMap::translate stopped identity-passing un-iomapped offsets
    // to match the RPCS3 oracle.
    rt.lv2_host_mut().seed_rsx_iomap(0, 0, 0x1000);
    rt
}

#[derive(Clone)]

struct RsxOffsetReleaseDriverUnit {
    id: UnitId,
    steps: Cell<u64>,
    fifo_base: u32,
    put_target: u32,
    sem_offset: u32,
    release_value: u32,
}

impl ExecutionUnit for RsxOffsetReleaseDriverUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use crate::rsx::control_register;
        use crate::rsx::method::{
            NV406E_SEMAPHORE_OFFSET, NV406E_SEMAPHORE_RELEASE, NV_COUNT_SHIFT,
        };
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= 2 {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        match n {
            1 => {
                let header_offset: u32 =
                    (1u32 << NV_COUNT_SHIFT) | (NV406E_SEMAPHORE_OFFSET as u32);
                let header_release: u32 =
                    (1u32 << NV_COUNT_SHIFT) | (NV406E_SEMAPHORE_RELEASE as u32);
                let words = [
                    header_offset,
                    self.sem_offset,
                    header_release,
                    self.release_value,
                ];
                let mut fifo_bytes: Vec<u8> = Vec::with_capacity(16);
                for w in words {
                    fifo_bytes.extend_from_slice(&w.to_be_bytes());
                }
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(self.fifo_base as u64), 16).unwrap(),
                    bytes: WritePayload::new(fifo_bytes),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(control_register::PUT_ADDR as u64), 4)
                        .unwrap(),
                    bytes: WritePayload::from_slice(&self.put_target.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
            }
            2 => {}
            _ => {}
        }
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn build_with_rsx_and_label_region(label_base: u32) -> Runtime {
    use cellgov_mem::{GuestMemory, PageSize, Region};
    let regions = vec![
        Region::new(0, 0x10000, "flat", PageSize::Page4K),
        Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
    ];
    let mem = GuestMemory::from_regions(regions).expect("non-overlapping");
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    rt.set_rsx_label_base(cellgov_mem::GuestAddr::new(label_base as u64));
    // Seed an identity iomap covering the flat region so the FIFO
    // advance pass can translate IO offsets back to EAs without
    // going through 672. Production code records this via
    // dispatch_sys_rsx_context_iomap; tests bypass the syscall and
    // record it directly. Required since IoMap::translate now
    // returns None for size == 0 (matching RPCS3's umax-on-miss),
    // so a runtime built without an iomap would surface
    // HeaderOutOfRange as soon as the consumer touches the FIFO.
    rt.lv2_host_mut().seed_rsx_iomap(0, 0, 0x10000);
    rt
}

/// Test fixture: emits a single ExecutionStepResult with
/// YieldReason::Syscall and caller-supplied syscall_args, then
/// reports Finished. Used by the apply_lv2_effects bypass tripwire
/// test below.
#[derive(Clone)]
struct Lv2SyscallEmitterUnit {
    id: UnitId,
    steps: Cell<u64>,
    syscall_args: [u64; 9],
}

impl ExecutionUnit for Lv2SyscallEmitterUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps.set(1);
        ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::with_pc(0x1000),
            fault: None,
            syscall_args: Some(self.syscall_args),
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Synthetic SPU-shape unit. Step 1 emits a `DmaEnqueue` with `tag_id=0`
/// and yields `DmaSubmitted`. Step 2+ checks `ctx.completed_dma_tags()`:
/// if bit 0 is set, accumulate `seen_tag_bits` and finish; else yield
/// `DmaWait`. Mirrors the production SPU's `MFC_PUT` + `MFC_RD_TAG_STAT`
/// pattern at the runtime level without needing an SPU ELF.
#[derive(Clone)]
struct TagPollUnit {
    id: UnitId,
    step: Cell<u32>,
    seen_tag_bits: Cell<u32>,
    dst_addr: u64,
}

impl ExecutionUnit for TagPollUnit {
    type Snapshot = u32;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.step.get() >= 3 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use cellgov_dma::{DmaDirection, DmaRequest};
        use cellgov_mem::{ByteRange, GuestAddr};
        let step = self.step.get();
        self.step.set(step + 1);
        match step {
            0 => {
                let src = ByteRange::new(GuestAddr::new(0x80), 4).unwrap();
                let dst = ByteRange::new(GuestAddr::new(self.dst_addr), 4).unwrap();
                let request = DmaRequest::new(DmaDirection::Put, src, dst, self.id)
                    .unwrap()
                    .with_tag_id(0);
                effects.push(Effect::DmaEnqueue {
                    request,
                    payload: Some(vec![0xAB; 4]),
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::DmaSubmitted,
                    consumed_cost: InstructionCost::new(1),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            _ => {
                let bits = ctx.completed_dma_tags();
                if bits & 1 != 0 {
                    self.seen_tag_bits.set(self.seen_tag_bits.get() | bits);
                    self.step.set(3);
                    ExecutionStepResult {
                        yield_reason: YieldReason::Finished,
                        consumed_cost: InstructionCost::new(1),
                        local_diagnostics: LocalDiagnostics::empty(),
                        fault: None,
                        syscall_args: None,
                    }
                } else {
                    ExecutionStepResult {
                        yield_reason: YieldReason::DmaWait,
                        consumed_cost: InstructionCost::new(budget.raw()),
                        local_diagnostics: LocalDiagnostics::empty(),
                        fault: None,
                        syscall_args: None,
                    }
                }
            }
        }
    }

    fn snapshot(&self) -> u32 {
        self.step.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommitErrShape {
    Reserved { addr: u64, region: &'static str },
    Other(String),
}

#[derive(Clone)]

struct RsxFlipSpinnerUnit {
    id: UnitId,
    steps: Cell<u64>,
    count: u64,
}

impl ExecutionUnit for RsxFlipSpinnerUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.count {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.count {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        effects.push(Effect::RsxFlipRequest {
            buffer_index: (n & 0x7) as u8,
        });
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn read_guest_u32_be(rt: &Runtime, addr: u32) -> u32 {
    let mem = rt.memory().as_bytes();
    let a = addr as usize;
    u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
}
