//! A supervisor thread returns by `rfid` with the SRR1 that the probe of
//! the `ppu_msr_le_readback` console capture wrote: the probe's MSR with LE
//! and PMM set. Its next `sc` saves the SRR1 that the reference console
//! saved, with PMM kept and LE 0.
//!
//! The console thread returns to problem state with translation on. This
//! thread stays in supervisor state in real mode, so the test removes PR,
//! IR and DR from both SRR1 values.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_SYSTEM_CALL;
use cellgov_ps3_abi::hw::ppu_supervisor::{
    MSR_DR, MSR_IR, MSR_LE, MSR_PMM, MSR_PR, MSR_RI, MSR_SF,
};
use cellgov_time::Budget;

const PROBE: &str = "ppu_msr_le_readback";

/// The real address of the `rfid`.
const CODE: u64 = 0x1000;

/// `rfid`, then `sc`.
const CODE_WORDS: [u32; 2] = [0x4C00_0024, 0x4400_0002];

/// `b .` at the system call vector.
const VECTOR_WORD: u32 = 0x4800_0000;

/// A supervisor MSR in real mode.
const KERNEL_MSR: u64 = MSR_SF | MSR_RI;

/// The words of the capture that the test reads.
struct Capture {
    /// The step that failed, or 0 when every step ran.
    step: u64,
    /// The SRR1 that the first poke wrote.
    written: u64,
    /// The SRR1 that the kernel saved at the next `sc`.
    saved: u64,
}

fn capture() -> Capture {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let payload = &frame[8..];
    assert_eq!(payload.len(), 192, "payload size");
    let word = |at: usize| u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"));
    Capture {
        step: word(0x00),
        written: word(0x50),
        saved: word(0x60),
    }
}

/// The SRR1 that the thread's `sc` saves after an `rfid` with SRR1
/// `srr1`.
fn cellgov_saved_srr1(srr1: u64) -> u64 {
    let mut mem = GuestMemory::new(0x2000);
    let mut put = |addr: u64, w: u32| {
        let range = ByteRange::new(GuestAddr::new(addr), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    };
    for (i, w) in CODE_WORDS.iter().enumerate() {
        put(CODE + 4 * i as u64, *w);
    }
    put(VECTOR_SYSTEM_CALL, VECTOR_WORD);
    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 10_000);
    let id = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let mut registers = SupervisorState::new(KERNEL_MSR);
        registers.srr0 = CODE + 4;
        registers.srr1 = srr1;
        let s = ppu.state_mut();
        s.set_supervisor(Some(registers));
        s.pc = CODE;
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
        if state(&rt).pc == VECTOR_SYSTEM_CALL {
            break;
        }
        let mut step = rt.step().expect("a runnable PPU thread");
        rt.commit_step(&mut step).expect("the step commits");
    }
    let s = state(&rt);
    let registers = s.supervisor().expect("supervisor registers");
    assert_eq!(
        s.pc, VECTOR_SYSTEM_CALL,
        "the sc reached its vector; SRR0 {:#x}, SRR1 {:#x}",
        registers.srr0, registers.srr1
    );
    assert_eq!(registers.srr0, CODE + 8, "SRR0 after the sc");
    registers.srr1
}

#[test]
fn an_rfid_that_restores_le_and_pmm_leaves_the_next_sc_saving_pmm_and_le_0_as_the_console_does() {
    let console = capture();
    assert_eq!(console.step, 0, "the probe ran every step");
    assert_ne!(console.written & MSR_LE, 0, "the console wrote LE");
    // PMM in the saved SRR1 shows that the written SRR1 reached the MSR.
    assert_ne!(console.saved & MSR_PMM, 0, "console PMM kept");
    let supervisor_real_mode = !(MSR_PR | MSR_IR | MSR_DR);
    assert_eq!(
        cellgov_saved_srr1(console.written & supervisor_real_mode),
        console.saved & supervisor_real_mode,
        "written {:#x}, console saved {:#x}",
        console.written,
        console.saved
    );
}
