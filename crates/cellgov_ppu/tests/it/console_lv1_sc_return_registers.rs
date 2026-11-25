//! A thread with supervisor registers in problem state makes the ten
//! `sc 1` calls of the `lv1_sc_return_registers` console capture through
//! the runtime's 4.93 hypervisor. Each call leaves r0, r3, r11, r12, CR
//! and XER as the reference console's lv1 leaves them. Each call past
//! the end of the table also leaves LR and CTR as the console does.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_EE, MSR_IR, MSR_PR, MSR_RI, MSR_SF};
use cellgov_ps3_abi::lv1::table::HYPERCALL_TABLE_ENTRIES;
use cellgov_time::Budget;

const PROBE: &str = "lv1_sc_return_registers";

/// The real address of the `sc 1`.
const CODE: u64 = 0x1000;

/// `sc 1`, then `b .`.
const WORDS: [u32; 2] = [0x4400_0022, 0x4800_0000];

/// A problem-state MSR in real mode.
const MSR: u64 = MSR_SF | MSR_PR | MSR_EE | MSR_RI;

/// The CR the console probe sets before each call.
const CR_IN: u32 = 0x6E7D_ACBF;

/// The size in bytes of one case in the payload: the number, the XER
/// sent, the address 20 bytes before the `sc`, then r0, r3, r11, r12, CR,
/// XER, LR and CTR.
const CASE_BYTES: usize = 88;

/// The registers the calls leave, in the order r0, r3, r11, r12, CR,
/// XER.
type Registers = [u64; 6];

/// One case as the console ran it.
struct ConsoleCase {
    number: u64,
    xer: u64,
    sc: u64,
    after: Registers,
    /// LR and CTR as the call returned them.
    lr_ctr: [u64; 2],
}

/// The mark that `ppu/main.c` puts in register `reg` before case `case`:
/// LR is register 32 and CTR is register 33.
fn mark(reg: u64, case: usize) -> u64 {
    0xC0DE_0000_0000_0000 | reg << 8 | case as u64
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
    assert_eq!(count, 10, "case count");
    assert_eq!(payload.len(), 8 + count * CASE_BYTES);
    (0..count)
        .map(|case| {
            let w = |i: usize| word(8 + case * CASE_BYTES + 8 * i);
            ConsoleCase {
                number: w(0),
                xer: w(1),
                sc: w(2) + 20,
                after: [w(3), w(4), w(5), w(6), w(7), w(8)],
                lr_ctr: [w(9), w(10)],
            }
        })
        .collect()
}

/// The registers, then LR and CTR, after the thread makes hypercall
/// `number` with XER `xer`.
///
/// Before the call, r0, r3 and r12 hold marks, and LR and CTR hold the
/// console probe's marks for case `case`.
fn cellgov_after(number: u64, xer: u64, case: usize) -> (Registers, [u64; 2]) {
    let mut mem = GuestMemory::new(0x2000);
    for (i, w) in WORDS.iter().enumerate() {
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
        s.set_gpr(12, 0xC0DE_0000_0000_0C00);
        s.set_cr(CR_IN);
        s.set_xer(xer);
        s.set_lr(mark(32, case));
        s.set_ctr(mark(33, case));
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
    for _ in 0..8 {
        if state(&rt).pc == CODE + 4 {
            break;
        }
        let mut step = rt.step().expect("a runnable PPU thread");
        rt.commit_step(&mut step).expect("the step commits");
    }
    let s = state(&rt);
    assert_eq!(
        s.pc,
        CODE + 4,
        "number {number:#x}: the thread stepped past sc"
    );
    let registers = [
        s.gpr[0],
        s.gpr[3],
        s.gpr[11],
        s.gpr[12],
        u64::from(s.cr()),
        s.xer(),
    ];
    (registers, [s.lr(), s.ctr()])
}

#[test]
fn each_hypercall_return_leaves_r0_r3_r11_r12_cr_and_xer_as_the_console_does() {
    let cases = console_cases();
    for (i, case) in cases.iter().enumerate() {
        let mut expected = case.after;
        if case.number < HYPERCALL_TABLE_ENTRIES {
            // The return through hrfid gives r11 and r12 the SRR0 and SRR1
            // of the caller's own sc. The console thread runs with
            // translation on, and this thread runs in real mode.
            assert_eq!(case.after[2], case.sc + 4, "case {i}: console r11");
            assert_eq!(
                case.after[3],
                MSR | MSR_IR | MSR_DR,
                "case {i}: console r12"
            );
            expected[2] = CODE + 4;
            expected[3] = MSR;
        }
        let (after, lr_ctr) = cellgov_after(case.number, case.xer, i);
        assert_eq!(
            after, expected,
            "case {i}: number {:#x}, XER {:#x}",
            case.number, case.xer
        );
        // The return through hrfid also changes LR and CTR, and CellGov
        // does not write those values. A call past the end of the table
        // returns through rfid, which leaves LR and CTR as the caller had
        // them.
        if case.number >= HYPERCALL_TABLE_ENTRIES {
            assert_eq!(lr_ctr, case.lr_ctr, "case {i}: LR and CTR");
        }
    }
}
