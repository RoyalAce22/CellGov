//! A hypercall's number never reaches the step loop's LV2 syscall
//! records.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use cellgov_compare::BootOutcome;
use cellgov_core::Runtime;
use cellgov_lv1::HypercallRefusal;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::instruction::encode::encode;
use cellgov_ppu::instruction::PpuInstruction as I;
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;
use cellgov_ps3_abi::lv2::syscall::{PPU_THREAD_EXIT, PROCESS_EXIT, TTY_WRITE};
use cellgov_time::Budget;

use crate::diag::{ProcessExitInfo, TtyCapture};
use crate::manifest::CheckpointTrigger;
use crate::step_loop::{step_loop, PcRing, StepLoopCtx, SyscallRing};
use crate::{BootSink, ChildInitPlans};

/// The step cap of every run: well past the longest program's step count.
const MAX_STEPS: usize = 64;

/// The address of the program's `sc`: the sixth word.
const SC_PC: u64 = 20;

/// Where the program's TTY text sits.
const TEXT_ADDR: u64 = 0x80;

/// The text r4 and r5 name, as a `sys_tty_write` buffer and length.
const TEXT: &[u8] = b"tty\n";

/// The first number the step loop counts as an HLE import index.
const HLE_BASE: u64 = 0x1_0000;

/// What a run wrote to each sink channel.
#[derive(Default)]
struct Narration {
    notes: RefCell<Vec<String>>,
    guest_text: RefCell<Vec<String>>,
}

impl BootSink for Narration {
    fn note(&self, line: &str) {
        self.notes.borrow_mut().push(line.to_string());
    }
    fn warn(&self, _line: &str) {}
    fn guest_text(&self, text: &str) {
        self.guest_text.borrow_mut().push(text.to_string());
    }
}

/// What a traced [`step_loop`] run left in its context, its sink and its
/// hypervisor.
struct Records {
    outcome: BootOutcome,
    last_exit: Option<ProcessExitInfo>,
    last_tty: Option<TtyCapture>,
    tty_oob_count: usize,
    syscalls: Vec<(u64, u64)>,
    hle_calls: BTreeMap<u32, usize>,
    /// The trace's lines for the call each step dispatched.
    call_lines: Vec<String>,
    guest_text: Vec<String>,
    refusal: Option<HypercallRefusal>,
}

/// `r3 = 0`, `r4 = TEXT_ADDR`, `r5 = TEXT.len()`, `r11 = number`, then
/// `sc lev` at [`SC_PC`] and a branch to itself.
fn program(number: u64, lev: u8) -> [I; 7] {
    let number = u32::try_from(number).expect("a 32-bit call number");
    [
        I::Addi {
            rt: 3,
            ra: 0,
            imm: 0,
        },
        I::Addi {
            rt: 4,
            ra: 0,
            imm: TEXT_ADDR as i16,
        },
        I::Addi {
            rt: 5,
            ra: 0,
            imm: TEXT.len() as i16,
        },
        I::Addis {
            rt: 11,
            ra: 0,
            imm: (number >> 16) as i16,
        },
        I::Ori {
            rs: 11,
            ra: 11,
            imm: number as u16,
        },
        I::Sc { lev },
        I::B {
            offset: 0,
            aa: false,
            link: false,
        },
    ]
}

/// Runs [`program`] through a traced [`step_loop`] on one PPU unit, with
/// supervisor registers when `supervisor` holds.
fn run(number: u64, lev: u8, supervisor: bool) -> Records {
    let mut mem = GuestMemory::new(0x100);
    let words = program(number, lev);
    for (i, insn) in words.iter().enumerate() {
        let w = encode(insn).expect("encodable");
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let text = ByteRange::new(GuestAddr::new(TEXT_ADDR), TEXT.len() as u64).expect("range");
    mem.apply_commit(text, TEXT).expect("mapped");
    let mut rt = Runtime::new(mem, Budget::new(1), MAX_STEPS);
    rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        if supervisor {
            unit.state_mut()
                .set_supervisor(Some(SupervisorState::new(MSR_SF)));
        }
        unit
    });

    let narration = Rc::new(Narration::default());
    let sink: Rc<dyn BootSink> = narration.clone();
    let child_init = ChildInitPlans::default();
    let mut steps = 0;
    let mut hle_calls = BTreeMap::new();
    let mut insn_coverage = BTreeMap::new();
    let mut pc_hits = BTreeMap::new();
    let mut timing = None;
    let mut ctx = StepLoopCtx {
        steps: &mut steps,
        hle_calls: &mut hle_calls,
        insn_coverage: &mut insn_coverage,
        trace: true,
        timing: &mut timing,
        loop_start: Instant::now(),
        pc_ring: PcRing::new(),
        last_tty: None,
        last_exit: None,
        syscall_ring: SyscallRing::new(),
        pc_hits: &mut pc_hits,
        checkpoint: CheckpointTrigger::ProcessExit,
        tty_oob_count: 0,
        bogus_fd_count: 0,
        dump_mem_fault_ranges: &[],
        obs_null_sink: false,
        child_init: &child_init,
        progress: &(),
        sink,
    };
    let (_, outcome) = step_loop(&mut rt, &mut ctx).expect("the loop runs");
    let last_exit = ctx.last_exit.take();
    let last_tty = ctx.last_tty.take();
    let tty_oob_count = ctx.tty_oob_count;
    let syscalls = ctx.syscall_ring.iter().collect();
    drop(ctx);
    let call_lines = narration
        .notes
        .borrow()
        .iter()
        .filter(|line| line.starts_with("       -> "))
        .cloned()
        .collect();
    let guest_text = narration.guest_text.borrow().clone();
    Records {
        outcome,
        last_exit,
        last_tty,
        tty_oob_count,
        syscalls,
        hle_calls,
        call_lines,
        guest_text,
        refusal: rt.hypervisor().refusal(),
    }
}

/// The refusal the hypervisor records for a hypercall `number` from the
/// one unit at [`SC_PC`]: a runtime with no firmware answers none.
fn refused(number: u64, records: &Records) -> bool {
    matches!(
        records.refusal,
        Some(HypercallRefusal::Unmodelled { number: n, pc: SC_PC, .. }) if n == number
    )
}

#[test]
fn a_hypercall_numbered_as_an_lv2_exit_records_no_exit_and_no_syscall() {
    for number in [PROCESS_EXIT, PPU_THREAD_EXIT] {
        let records = run(number, 1, true);
        assert!(refused(number, &records), "hypercall {number} reached lv1");
        assert_eq!(records.outcome, BootOutcome::Fault, "hypercall {number}");
        assert!(records.last_exit.is_none(), "hypercall {number}");
        assert_eq!(records.syscalls, [], "hypercall {number}");
    }
}

#[test]
fn a_hypercall_numbered_as_sys_tty_write_captures_no_tty_text() {
    let records = run(TTY_WRITE, 1, true);
    assert!(refused(TTY_WRITE, &records));
    assert!(records.last_tty.is_none());
    assert_eq!(records.tty_oob_count, 0);
    assert_eq!(records.syscalls, []);
}

#[test]
fn a_hypercall_number_at_the_hle_base_counts_no_hle_call() {
    let records = run(HLE_BASE, 1, true);
    assert!(refused(HLE_BASE, &records));
    assert_eq!(records.hle_calls, BTreeMap::new());
    assert_eq!(records.syscalls, []);
}

#[test]
fn the_trace_names_a_hypercall_by_its_number_and_echoes_no_guest_text() {
    let records = run(TTY_WRITE, 1, true);
    assert!(refused(TTY_WRITE, &records));
    assert_eq!(records.call_lines, ["       -> hypercall 403"]);
    assert_eq!(records.guest_text, Vec::<String>::new());
}

#[test]
fn an_lv2_sys_process_exit_records_the_exit_and_the_syscall() {
    let records = run(PROCESS_EXIT, 0, false);
    assert_eq!(records.refusal, None);
    assert_eq!(records.outcome, BootOutcome::ProcessExit);
    assert_eq!(
        records.last_exit.map(|exit| (exit.code, exit.call_pc)),
        Some((0, SC_PC))
    );
    assert_eq!(records.syscalls, [(PROCESS_EXIT, SC_PC)]);
    assert_eq!(records.call_lines, ["       -> LV2 syscall 22"]);
}

#[test]
fn an_lv2_sys_tty_write_captures_the_text() {
    let records = run(TTY_WRITE, 0, false);
    assert_eq!(records.refusal, None);
    assert_eq!(
        records
            .last_tty
            .map(|tty| (tty.fd, tty.raw_bytes, tty.call_pc)),
        Some((0, TEXT.to_vec(), SC_PC))
    );
    assert_eq!(records.syscalls, [(TTY_WRITE, SC_PC)]);
}
