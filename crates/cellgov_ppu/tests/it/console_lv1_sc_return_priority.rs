//! A thread with supervisor registers in problem state makes the eight
//! `sc 1` calls of the `lv1_sc_return_priority` console capture. Each
//! call follows the console capture's priority form, and the runtime's
//! 4.93 hypervisor answers it. Each call leaves r0 and r3 as the
//! reference console's lv1 leaves them.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_EE, MSR_PR, MSR_RI, MSR_SF};
use cellgov_time::Budget;

const PROBE: &str = "lv1_sc_return_priority";

/// The real address of the priority form.
const CODE: u64 = 0x1000;

/// `ori 0,0,0`, the nop with no side effect.
const NOP: u32 = 0x6000_0000;

/// `sc 1`.
const SC1: u32 = 0x4400_0022;

/// `b .`.
const HALT: u32 = 0x4800_0000;

/// A problem-state MSR in real mode.
const MSR: u64 = MSR_SF | MSR_PR | MSR_EE | MSR_RI;

/// The size in bytes of one case in the payload: the number, the form,
/// then r0 and r3.
const CASE_BYTES: usize = 32;

/// One case as the console ran it.
struct ConsoleCase {
    number: u64,
    form: u64,
    r0: u64,
    r3: u64,
}

/// `or n,n,n`: the form that asks for priority `n` (1 low, 2 medium, 3
/// high), or [`NOP`] for form 0.
fn priority_form(form: u64) -> u32 {
    assert!(form <= 3, "form {form}");
    if form == 0 {
        return NOP;
    }
    let r = form as u32;
    0x7C00_0378 | r << 21 | r << 16 | r << 11
}

fn console_cases() -> Vec<ConsoleCase> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let payload = &frame[8..];
    let word = |at: usize| u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"));
    let count = word(0) as usize;
    assert_eq!(count, 8, "case count");
    assert_eq!(payload.len(), 8 + count * CASE_BYTES);
    (0..count)
        .map(|case| {
            let w = |i: usize| word(8 + case * CASE_BYTES + 8 * i);
            ConsoleCase {
                number: w(0),
                form: w(1),
                r0: w(2),
                r3: w(3),
            }
        })
        .collect()
}

/// r0 and r3 after the thread runs priority form `form`, hypercall
/// `number` and `or 2,2,2`.
///
/// Before the call, r0 and r3 hold marks.
fn cellgov_after(number: u64, form: u64) -> [u64; 2] {
    let words = [priority_form(form), SC1, priority_form(2), HALT];
    let mut mem = GuestMemory::new(0x2000);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 10_000);
    rt.set_hypervisor_firmware("4.93");
    let id = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let s = ppu.state_mut();
        s.set_supervisor(Some(SupervisorState::new(MSR)));
        s.pc = CODE;
        s.set_gpr(0, 0xC0DE_0000_0000_0000);
        s.set_gpr(3, 0xC0DE_0000_0000_0300);
        s.set_gpr(11, number);
        ppu
    });
    let state = |rt: &cellgov_core::Runtime| {
        rt.registry()
            .get(id)
            .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
            .expect("the PPU unit")
            .state()
            .clone()
    };
    let end = CODE + 12;
    for _ in 0..16 {
        if state(&rt).pc == end {
            break;
        }
        let mut step = rt.step().expect("a runnable PPU thread");
        rt.commit_step(&mut step).expect("the step commits");
    }
    let s = state(&rt);
    assert_eq!(
        s.pc, end,
        "number {number:#x}, form {form}: the thread ran past the sc"
    );
    [s.gpr[0], s.gpr[3]]
}

#[test]
fn each_hypercall_leaves_r0_and_r3_as_the_console_does_after_each_priority_form() {
    let cases = console_cases();
    for (i, case) in cases.iter().enumerate() {
        assert_eq!(
            cellgov_after(case.number, case.form),
            [case.r0, case.r3],
            "case {i}: number {:#x}, form {}",
            case.number,
            case.form
        );
    }
}
