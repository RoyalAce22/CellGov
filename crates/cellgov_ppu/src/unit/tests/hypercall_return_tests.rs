use super::super::hypercall::take_compare_fields;
use super::interrupt_support::*;
use cellgov_exec::{CompareFields, CompareResult, ExecutionUnit, SaveRestore, YieldReason};
use cellgov_ps3_abi::hw::ppu_interrupt::SRR1_INTERRUPT_INFO;
use cellgov_ps3_abi::lv1::table::HYPERCALL_TABLE_ENTRIES;

/// The CR that the capture's probe sets before each call.
const CR_IN: u32 = 0x6E7D_ACBF;

/// The bytes of one case in the `lv1_sc_return_registers` payload: the
/// number, the XER sent, an address, then r0, r3, r11, r12, CR, XER, LR
/// and CTR.
const CASE_BYTES: usize = 88;

/// One call of the `lv1_sc_return_registers` capture that lv1 answered
/// through `hrfid`.
struct HrfidCase {
    /// The XER the probe sent.
    xer: u64,
    /// r12 after the call: the SRR1 of the probe's `sc`.
    r12: u64,
    /// CR after the call.
    cr: u32,
}

/// The capture's calls with a number in the hypercall table, in
/// payload order.
fn console_hrfid_cases() -> Vec<HrfidCase> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv1_sc_return_registers/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let payload = &frame[8..];
    let word = |at: usize| u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"));
    let count = word(0) as usize;
    assert_eq!(count, 10, "case count");
    assert_eq!(payload.len(), 8 + count * CASE_BYTES);
    let cases: Vec<HrfidCase> = (0..count)
        .map(|case| {
            let w = |i: usize| word(8 + case * CASE_BYTES + 8 * i);
            (w(0), w(1), w(6), w(7))
        })
        .filter(|&(number, ..)| number < HYPERCALL_TABLE_ENTRIES)
        .map(|(_, xer, r12, cr)| HrfidCase {
            xer,
            r12,
            cr: u32::try_from(cr).expect("a 32-bit CR"),
        })
        .collect();
    assert_eq!(cases.len(), 4, "calls into the table");
    cases
}

/// The CR fields of lv1's return through `hrfid` to a problem-state
/// caller: EQ in CR0 and CR1, GT in CR7.
fn hrfid_problem_state() -> CompareFields {
    let mut cr = [None; 8];
    cr[0] = Some(CompareResult::Equal);
    cr[1] = Some(CompareResult::Equal);
    cr[7] = Some(CompareResult::Greater);
    CompareFields(cr)
}

#[test]
fn a_hypercall_gives_the_address_after_sc_and_the_msr_as_its_save_restore_pair() {
    let mem = memory(&[SC_1, NOP]);
    let mut unit = kernel_unit(KERNEL_MSR);
    let (r, _) = run(&mut unit, &mem, 4);
    assert_eq!(r.yield_reason, YieldReason::Syscall);
    assert_eq!(
        unit.system_call_save_restore(),
        Some(SaveRestore {
            srr0: CODE + 4,
            srr1: KERNEL_MSR,
        })
    );
}

#[test]
fn the_save_restore_pair_clears_the_interrupt_bits_of_srr1() {
    let console_msr = console_hrfid_cases()[0].r12;
    assert_eq!(console_msr & SRR1_INTERRUPT_INFO, 0, "console r12");
    let mut unit = kernel_unit(console_msr | SRR1_INTERRUPT_INFO);
    unit.state_mut().pc = CODE;
    let pair = unit
        .system_call_save_restore()
        .expect("supervisor registers");
    assert_eq!(pair.srr1, console_msr);
}

#[test]
fn a_thread_with_no_supervisor_registers_gives_no_save_restore_pair() {
    let unit = crate::PpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    assert_eq!(unit.system_call_save_restore(), None);
}

#[test]
fn compare_fields_take_xer_so_as_their_so_bit_and_leave_the_other_fields() {
    for (i, case) in console_hrfid_cases().iter().enumerate() {
        let mut unit = kernel_unit(KERNEL_MSR);
        let state = unit.state_mut();
        state.set_cr(CR_IN);
        state.set_xer(case.xer);
        take_compare_fields(state, hrfid_problem_state());
        assert_eq!(state.cr(), case.cr, "call {i}: XER {:#x}", case.xer);
        assert_eq!(state.xer(), case.xer, "call {i}: XER {:#x}", case.xer);
    }
}

/// [PPC-Book1 p:60 s:3.3.9] A compare puts LT, GT or EQ in bits 0 to 2
/// of its CR field and XER\[SO\] in bit 3.
#[test]
fn a_less_compare_field_takes_lt_and_xer_so_and_leaves_the_other_fields() {
    for (xer, expected) in [(0, 0x687D_ACBF), (0x8000_0000, 0x697D_ACBF)] {
        let mut unit = kernel_unit(KERNEL_MSR);
        let state = unit.state_mut();
        state.set_cr(CR_IN);
        state.set_xer(xer);
        let mut cr = [None; 8];
        cr[1] = Some(CompareResult::Less);
        take_compare_fields(state, CompareFields(cr));
        assert_eq!(state.cr(), expected, "XER {xer:#x}");
    }
}
