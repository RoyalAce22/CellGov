//! The `sc` words of the `ppu_sc_reserved_bits` console capture, run on a
//! thread with supervisor registers.
//!
//! On the console each word gives the plain `sc`'s answers, so the PPE
//! takes the same system call interrupt for each word.

use crate::console_ppu_sc_reserved_bits::{console, same_as_plain, WORDS};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, InterruptRecord, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_SYSTEM_CALL;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_ME, MSR_RI, MSR_SF};
use cellgov_time::Budget;

/// The real address of the `sc` word.
const CODE: u64 = 0x2_0000;

/// What a thread with supervisor registers does at one `sc` word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    yield_reason: YieldReason,
    interrupt: Option<InterruptRecord>,
    pc: u64,
    msr: u64,
}

/// Runs `word` in real mode on a thread with supervisor registers, for
/// one instruction.
fn take(word: u32) -> Taken {
    let mut mem = GuestMemory::new(0x4_0000);
    let range = ByteRange::new(GuestAddr::new(CODE), 4).expect("a range");
    mem.apply_commit(range, &word.to_be_bytes())
        .expect("the memory holds the word");
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut()
        .set_supervisor(Some(SupervisorState::new(MSR_SF | MSR_ME | MSR_RI)));
    unit.state_mut().pc = CODE;
    let r = unit.run_until_yield(
        Budget::new(1),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    let s = unit.state();
    Taken {
        yield_reason: r.yield_reason,
        interrupt: r.local_diagnostics.interrupt,
        pc: s.pc,
        msr: s.supervisor().expect("supervisor registers").msr,
    }
}

#[test]
fn each_sc_word_the_console_runs_as_a_plain_sc_takes_the_system_call_vector() {
    let console = console();
    assert_eq!(console[0].word, 0x4400_0002, "the first word is a plain sc");
    let taken = console.map(|a| take(a.word));
    let plain = taken[0];
    assert_eq!(plain.yield_reason, YieldReason::InterruptBoundary);
    let record = plain.interrupt.expect("an interrupt record");
    assert_eq!(
        (record.vector, record.srr0, plain.pc),
        (VECTOR_SYSTEM_CALL, CODE + 4, VECTOR_SYSTEM_CALL),
        "the plain sc enters the system call vector"
    );
    let cellgov: [bool; WORDS] = std::array::from_fn(|i| taken[i] == plain);
    assert_eq!(cellgov, same_as_plain(&console), "{taken:#x?}");
}
