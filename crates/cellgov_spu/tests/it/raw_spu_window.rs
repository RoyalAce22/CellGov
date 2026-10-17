//! A PPU drives a Raw SPU through its problem-state window:
//!
//! - plain stores load the program and start it
//! - plain loads read its state back
//! - the Raw SPU syscalls read the interrupts and the interrupt mailbox
//!   message that the run left
//!
//! The expected words come from the spu_raw_lifecycle console capture.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use std::cell::Cell;

use cellgov_core::{AddressSpaceId, Runtime, RuntimeMode};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::hw::spu::{
    SPU_IN_MBOX_OFFSET, SPU_MBOX_STAT_OFFSET, SPU_NPC_OFFSET, SPU_OUT_MBOX_OFFSET,
    SPU_RUN_CNTL_OFFSET, SPU_SIG_NOTIFY_1_OFFSET, SPU_STATUS_OFFSET,
};
use cellgov_ps3_abi::lv2::spu::raw_window;
use cellgov_ps3_abi::lv2::syscall::{
    RAW_SPU_CREATE, RAW_SPU_DESTROY, RAW_SPU_GET_INT_STAT, RAW_SPU_READ_PUINT_MB,
    RAW_SPU_SET_SPU_CFG, SPU_INITIALIZE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

/// One step of the scripted PPU.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    Call([u64; 4]),
    Store(u64, u32),
    Load(u64),
    /// Load the word at the address, and run this op again until the
    /// word ANDed with the mask equals the value.
    Poll(u64, u32, u32),
}

/// The probe's poll of `SPU_Status` until R clears.
pub(crate) fn poll_stopped() -> Op {
    Op::Poll(register(SPU_STATUS_OFFSET), 1, 0)
}

/// Runs one [`Op`] per step, then finishes.
#[derive(Clone)]
struct Ppu {
    id: UnitId,
    ops: Vec<Op>,
    steps: Cell<usize>,
}

impl ExecutionUnit for Ppu {
    type Snapshot = usize;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.ops.len() {
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
        let op = self.ops[self.steps.get()];
        let polling = match op {
            Op::Poll(addr, mask, want) => {
                let held = ctx.memory().read(word(addr)).unwrap();
                u32::from_be_bytes(held.try_into().unwrap()) & mask != want
            }
            _ => false,
        };
        if !polling {
            self.steps.set(self.steps.get() + 1);
        }
        let mut result = ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::with_pc(0x1000),
            fault: None,
            syscall_args: None,
        };
        match op {
            Op::Call(call) => {
                let mut args = [0u64; 9];
                args[..4].copy_from_slice(&call);
                result.yield_reason = YieldReason::Syscall;
                result.syscall_args = Some(args);
            }
            Op::Store(addr, value) => effects.push(Effect::shared_write(
                word(addr),
                WritePayload::from_slice(&value.to_be_bytes()),
                self.id,
                GuestTicks::ZERO,
            )),
            Op::Load(addr) | Op::Poll(addr, ..) => effects.push(Effect::SharedReadIntent {
                range: word(addr),
                source: self.id,
            }),
        }
        result
    }

    fn snapshot(&self) -> usize {
        self.steps.get()
    }
}

fn word(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), 4).unwrap()
}

pub(crate) const OUT_PTR: u64 = 0x800;
const GATE: u64 = 0x900;
pub(crate) const WINDOW: u64 = raw_window::BASE;

pub(crate) fn register(offset: u32) -> u64 {
    WINDOW + raw_window::PROBLEM_STATE + u64::from(offset)
}

/// spu/main.S of spu_raw_lifecycle:
///
/// 1. Read the inbound mailbox and add one.
/// 2. Write the sum to the outbound mailbox.
/// 3. Write 0x1EEF to the interrupt mailbox.
/// 4. Stop with 0x100.
const PROGRAM: [u32; 7] = [
    0x01a0_0e83,
    0x1c00_4183,
    0x21a0_0e03,
    0x408f_7784,
    0x21a0_0f04,
    0x0000_0100,
    0x0000_2222,
];

/// A runtime with main memory, the reserved SPU range, and one PPU that
/// runs `ops`.
pub(crate) fn runtime(ops: Vec<Op>) -> (Runtime, UnitId) {
    let memory = GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            raw_window::BASE,
            2 * raw_window::STRIDE as usize,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let mut rt = Runtime::new(memory, Budget::new(1), 400);
    rt.set_spu_factory(|id, _| Ok(Box::new(SpuExecutionUnit::new(id))));
    let ppu = rt.register_unit_with(|id| Ppu {
        id,
        ops,
        steps: Cell::new(0),
    });
    (rt, ppu)
}

/// The number of ops the PPU finished.
fn ops_done(rt: &Runtime, ppu: UnitId) -> usize {
    let unit = rt.registry().get(ppu).unwrap().as_any();
    unit.downcast_ref::<Ppu>().unwrap().steps.get()
}

/// Step and commit until the PPU finishes `n` more ops.
pub(crate) fn run_ops(rt: &mut Runtime, ppu: UnitId, n: usize) {
    let target = ops_done(rt, ppu) + n;
    for _ in 0..200 {
        if ops_done(rt, ppu) >= target {
            return;
        }
        let step = rt.step().expect("a unit steps");
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    panic!("the PPU finished {} of {target} ops", ops_done(rt, ppu));
}

/// Step and commit until no unit can step.
fn run_out(rt: &mut Runtime) {
    for _ in 0..200 {
        let Ok(step) = rt.step() else {
            return;
        };
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
}

pub(crate) fn read(rt: &Runtime, addr: u64) -> u32 {
    let bytes = rt.memory().read(word(addr)).unwrap();
    u32::from_be_bytes(bytes.try_into().unwrap())
}

pub(crate) fn create_ops() -> Vec<Op> {
    vec![
        Op::Call([SPU_INITIALIZE, 6, 1, 0]),
        Op::Call([RAW_SPU_CREATE, OUT_PTR, 0, 0]),
    ]
}

#[test]
fn a_raw_spu_runs_through_its_window_as_the_console_did() {
    let mut ops = create_ops();
    ops.extend(
        PROGRAM
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + 4 * i as u64, w)),
    );
    ops.extend([
        Op::Store(register(SPU_IN_MBOX_OFFSET), 0x41),
        Op::Store(register(SPU_NPC_OFFSET), 0),
        Op::Store(register(SPU_RUN_CNTL_OFFSET), 1),
    ]);
    let (mut rt, ppu) = runtime(ops);

    run_ops(&mut rt, ppu, 2);
    assert_eq!(read(&rt, OUT_PTR), 0, "the create writes id 0");
    assert_eq!(read(&rt, register(SPU_STATUS_OFFSET)), 0, "w38: stopped");
    assert_eq!(read(&rt, register(SPU_MBOX_STAT_OFFSET)), 0x400, "w39");

    run_ops(&mut rt, ppu, PROGRAM.len() + 1);
    let raw = rt.lv2_host().raw_spu_unit(0).unwrap();
    let spu = rt.registry().get(raw).unwrap().as_any();
    let ls = &spu.downcast_ref::<SpuExecutionUnit>().unwrap().state().ls;
    assert_eq!(
        ls[..4],
        PROGRAM[0].to_be_bytes(),
        "the stores land in the local store"
    );
    assert_eq!(read(&rt, register(SPU_MBOX_STAT_OFFSET)), 0x300, "w41");

    run_out(&mut rt);
    assert_eq!(
        read(&rt, register(SPU_STATUS_OFFSET)),
        0x0100_0002,
        "w43: stopped with 0x100"
    );
    assert_eq!(read(&rt, register(SPU_NPC_OFFSET)), 0x18, "w44");
    assert_eq!(read(&rt, register(SPU_MBOX_STAT_OFFSET)), 0x10401, "w45");
    assert_eq!(read(&rt, register(SPU_OUT_MBOX_OFFSET)), 0x42, "w46");
}

/// Runs the program of [`a_raw_spu_runs_through_its_window_as_the_console_did`]
/// to the SPU's stop, and leaves the `tail` ops for the caller to run.
fn after_the_run(tail: Vec<Op>) -> (Runtime, UnitId) {
    let mut ops = create_ops();
    ops.extend(
        PROGRAM
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + 4 * i as u64, w)),
    );
    ops.extend([
        Op::Store(register(SPU_IN_MBOX_OFFSET), 0x41),
        Op::Store(register(SPU_RUN_CNTL_OFFSET), 1),
        poll_stopped(),
    ]);
    let run = ops.len();
    ops.extend(tail);
    let (mut rt, ppu) = runtime(ops);
    run_ops(&mut rt, ppu, run);
    (rt, ppu)
}

#[test]
fn a_load_of_the_outbound_mailbox_takes_its_message() {
    let (mut rt, ppu) = after_the_run(vec![Op::Load(register(SPU_OUT_MBOX_OFFSET))]);
    run_ops(&mut rt, ppu, 1);
    assert_eq!(read(&rt, register(SPU_MBOX_STAT_OFFSET)), 0x10400, "w47");
    assert_eq!(
        read(&rt, register(SPU_OUT_MBOX_OFFSET)),
        0x42,
        "an empty mailbox keeps the word it last showed"
    );
    assert_problem_state_reached(&rt);
}

/// Asserts that the last step reached the Raw SPU's problem-state area,
/// as the schedule explorer reads it.
fn assert_problem_state_reached(rt: &Runtime) {
    let raw = rt.lv2_host().raw_spu_unit(0).unwrap();
    assert!(
        rt.last_problem_state_accesses()
            .iter()
            .any(|(unit, range)| *unit == raw && range.start().raw() == raw_window::PROBLEM_STATE),
        "{:?}",
        rt.last_problem_state_accesses()
    );
}

#[test]
fn the_run_latches_s_and_m_and_the_puint_read_takes_the_message() {
    let (mut rt, ppu) = after_the_run(vec![
        Op::Call([RAW_SPU_GET_INT_STAT, 0, 2, OUT_PTR]),
        Op::Call([RAW_SPU_READ_PUINT_MB, 0, OUT_PTR, 0]),
        Op::Call([RAW_SPU_READ_PUINT_MB, 0, OUT_PTR + 8, 0]),
    ]);
    run_ops(&mut rt, ppu, 1);
    assert_eq!(read(&rt, OUT_PTR + 4), 0x13, "w49: B, S and M");
    run_ops(&mut rt, ppu, 1);
    assert_eq!(read(&rt, OUT_PTR), 0x1EEF, "w51");
    assert_problem_state_reached(&rt);
    assert_eq!(
        read(&rt, register(SPU_MBOX_STAT_OFFSET)),
        0x401,
        "w52 less the load"
    );
    run_ops(&mut rt, ppu, 1);
    assert_eq!(read(&rt, OUT_PTR + 8), 0x1EEF, "w60: the register keeps it");
}

#[test]
fn a_raw_spu_step_projects_its_local_store_writes() {
    // il r3, 0x77; stqd r3, 0x100(r0); stop.
    let program = [0x4080_3b83u32, 0x2404_0003, 0x0000_0000];
    let mut ops = create_ops();
    ops.extend(
        program
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + 4 * i as u64, w)),
    );
    ops.push(Op::Store(register(SPU_RUN_CNTL_OFFSET), 1));
    let (mut rt, _) = runtime(ops);
    run_out(&mut rt);
    assert_eq!(read(&rt, WINDOW + 0x100), 0x77);
}

/// Boot runs in FaultDriven mode, whose fast commit path must still
/// project a Raw SPU step that emits no effect.
#[test]
fn a_raw_spu_that_spins_after_a_store_shows_it_in_fault_driven_mode() {
    // il r3, 0x77; stqd r3, 0x100(r0); br self.
    let program = [0x4080_3b83u32, 0x2404_0003, 0x3200_0000];
    let mut ops = create_ops();
    ops.extend(
        program
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + 4 * i as u64, w)),
    );
    ops.push(Op::Store(register(SPU_RUN_CNTL_OFFSET), 1));
    ops.push(Op::Poll(WINDOW + 0x100, u32::MAX, 0x77));
    let n = ops.len();
    let (mut rt, ppu) = runtime(ops);
    rt.set_mode(RuntimeMode::FaultDriven);
    run_ops(&mut rt, ppu, n);
}

/// `il rt, imm`.
fn il(rt: u32, imm: u32) -> u32 {
    (0x081 << 23) | (imm << 7) | rt
}

/// `wrch channel, rt`.
fn wrch(channel: u32, rt: u32) -> u32 {
    (0x10D << 21) | (channel << 7) | rt
}

#[test]
fn a_get_that_lands_after_the_raw_spu_stopped_reaches_the_window() {
    // A 16-byte get from main storage 0x100 to local store 0x200, then
    // a stop that does not wait for it.
    let program = [
        il(3, 0x200),
        wrch(16, 3),
        il(4, 0x100),
        wrch(18, 4),
        il(5, 16),
        wrch(19, 5),
        il(6, 0),
        wrch(20, 6),
        il(7, 0x40),
        wrch(21, 7),
        0x0000_0000,
    ];
    let mut ops = create_ops();
    ops.push(Op::Store(0x100, 0xABCD_1234));
    ops.extend(
        program
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + 4 * i as u64, w)),
    );
    ops.push(Op::Store(register(SPU_RUN_CNTL_OFFSET), 1));
    ops.push(poll_stopped());
    // The PPU continues to run while the transfer is in flight.
    ops.extend([Op::Load(0x100); 40]);
    let (mut rt, _) = runtime(ops);
    run_out(&mut rt);
    let raw = rt.lv2_host().raw_spu_unit(0).unwrap();
    assert!(rt.unit_stop_registers(raw).is_some(), "the SPU stopped");
    let spu = rt.registry().get(raw).unwrap().as_any();
    let ls = &spu.downcast_ref::<SpuExecutionUnit>().unwrap().state().ls;
    assert_eq!(
        ls[0x200..0x204],
        0xABCD_1234u32.to_be_bytes(),
        "the get landed"
    );
    assert_eq!(read(&rt, WINDOW + 0x200), 0xABCD_1234);
}

#[test]
fn a_full_interrupt_mailbox_stalls_the_raw_spu_until_the_puint_read() {
    let program = [
        il(4, 0x11),
        wrch(30, 4),
        il(4, 0x22),
        wrch(30, 4),
        0x0000_0000,
    ];
    let mut ops = create_ops();
    ops.extend(
        program
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + 4 * i as u64, w)),
    );
    ops.push(Op::Store(register(SPU_RUN_CNTL_OFFSET), 1));
    // The PPU waits on a word the test places after the SPU parks.
    ops.push(Op::Poll(GATE, 1, 1));
    let run = ops.len();
    ops.extend([
        Op::Call([RAW_SPU_READ_PUINT_MB, 0, OUT_PTR, 0]),
        poll_stopped(),
        Op::Call([RAW_SPU_READ_PUINT_MB, 0, OUT_PTR + 4, 0]),
    ]);
    let (mut rt, ppu) = runtime(ops);
    run_ops(&mut rt, ppu, run - 1);
    let raw = rt.lv2_host().raw_spu_unit(0).unwrap();
    for _ in 0..40 {
        if rt.registry().effective_status(raw) == Some(UnitStatus::Blocked) {
            break;
        }
        let step = rt.step().unwrap();
        rt.commit_step(&step.result, &step.effects).unwrap();
    }
    assert_eq!(
        rt.registry().effective_status(raw),
        Some(UnitStatus::Blocked),
        "the second write waits for the slot"
    );
    rt.place_bytes(AddressSpaceId::BOOT, word(GATE), &1u32.to_be_bytes())
        .unwrap();
    run_ops(&mut rt, ppu, 4);
    assert_eq!(read(&rt, OUT_PTR), 0x11);
    assert_eq!(read(&rt, OUT_PTR + 4), 0x22, "the read woke the write");
}

/// `rdch rt, channel`.
fn rdch(rt: u32, channel: u32) -> u32 {
    (0x00D << 21) | (channel << 7) | rt
}

/// The word a Raw SPU reads from signal notification 1 after two
/// window stores of 0b01 and 0b10, under `SPU_Cfg` `cfg`.
fn signal_after_two_stores(cfg: u64) -> u32 {
    // Read signal 1, write it to the outbound mailbox, stop.
    let program = [rdch(3, 3), wrch(28, 3), 0x0000_0000];
    let mut ops = create_ops();
    ops.push(Op::Call([RAW_SPU_SET_SPU_CFG, 0, cfg, 0]));
    ops.extend(
        program
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + 4 * i as u64, w)),
    );
    ops.extend([
        Op::Store(register(SPU_SIG_NOTIFY_1_OFFSET), 0b01),
        Op::Store(register(SPU_SIG_NOTIFY_1_OFFSET), 0b10),
        Op::Store(register(SPU_RUN_CNTL_OFFSET), 1),
        poll_stopped(),
    ]);
    let total = ops.len();
    let (mut rt, ppu) = runtime(ops);
    run_ops(&mut rt, ppu, total);
    read(&rt, register(SPU_OUT_MBOX_OFFSET))
}

#[test]
fn spu_cfg_sets_how_a_raw_spus_signal_register_takes_a_window_store() {
    assert_eq!(signal_after_two_stores(0), 0b10, "overwrite");
    assert_eq!(signal_after_two_stores(1), 0b11, "S1 set: logical OR");
}

#[test]
fn a_store_to_spu_npc_of_a_stopped_raw_spu_sets_where_it_resumes() {
    let mut ops = create_ops();
    ops.push(Op::Store(register(SPU_NPC_OFFSET), 0x40));
    let (mut rt, ppu) = runtime(ops);
    run_ops(&mut rt, ppu, 3);
    let raw = rt.lv2_host().raw_spu_unit(0).unwrap();
    assert_eq!(rt.unit_stop_registers(raw).map(|r| r.npc), Some(0x40));
}

#[test]
fn a_destroy_returns_the_window_to_the_reserved_range() {
    let mut ops = create_ops();
    ops.push(Op::Store(WINDOW, 0x1234));
    ops.push(Op::Call([RAW_SPU_DESTROY, 0, 0, 0]));
    let (mut rt, ppu) = runtime(ops);
    let layout = |rt: &Runtime| -> Vec<_> {
        rt.memory()
            .regions()
            .map(|r| (r.base(), r.label(), r.access()))
            .collect()
    };
    let before = layout(&rt);
    run_ops(&mut rt, ppu, 3);
    assert_ne!(layout(&rt), before, "the window is mapped");
    run_ops(&mut rt, ppu, 1);
    assert_eq!(layout(&rt), before);
    assert_eq!(read(&rt, WINDOW), 0, "the reserved range reads zero again");
}
