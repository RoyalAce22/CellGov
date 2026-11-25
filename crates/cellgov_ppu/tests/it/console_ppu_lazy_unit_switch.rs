//! Checks the unavailable interrupts of a thread with supervisor registers
//! against the `ppu_lazy_unit_switch` and `ppu_lazy_unit_switch_vrsave`
//! console captures.
//!
//! On the console, the first floating-point add and the first vector add
//! after a thread switch each cost an interrupt. The same add, run again at
//! once, costs none. The kernel resumes a thread with MSR\[FP\] and
//! MSR\[VXU\] clear, so the threads of the real LV2 kernel take the
//! floating-point and VXU unavailable interrupts.

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_interrupt::{VECTOR_FP_UNAVAILABLE, VECTOR_VXU_UNAVAILABLE};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_FP, MSR_ME, MSR_RI, MSR_SF, MSR_VXU};
use cellgov_time::Budget;

/// The time-base steps that an interrupt adds to the cost of an instruction.
///
/// This is about 400 processor cycles, more than any integer or warm
/// instruction of either capture costs.
const INTERRUPT_TICKS: u32 = 10;

/// The real address of the first timed word.
const CODE: u64 = 0x2_0000;

/// `add r10,r10,r10`, one of the three forms the `ppu_lazy_unit_switch`
/// probe times. The compiler of the probe picks the registers of this add
/// and of `FADD`.
const ADD: u32 = 0x7D4A_5214;
/// `fadd f0,f0,f0`, which the probe times.
const FADD: u32 = 0xFC00_002A;
/// `vaddubm v1,v2,v3`, which the probe times.
const VADDUBM: u32 = 0x1022_1800;

/// The payload words of the capture of `probe`, after its frame header.
fn payload(probe: &str, bytes: usize) -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "{probe}: frame magic");
    let p = &frame[8..];
    assert_eq!(p.len(), bytes, "{probe}");
    let words: Vec<u32> = p
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!(words[0], 0, "{probe}: no step failed");
    words
}

/// The six deltas of each `ppu_lazy_unit_switch` round: integer,
/// floating-point and vector, each first and then warm.
fn lazy_rounds() -> Vec<[u32; 6]> {
    /// The value of word 2 after every helper thread ran.
    const HELPER_DONE: u32 = 0x600D;
    /// The first word of the rounds of each section, and the word that
    /// holds its round count.
    const SECTIONS: [(usize, usize); 2] = [(8, 3), (200, 4)];
    let w = payload("ppu_lazy_unit_switch", 1184);
    assert_eq!(w[2], HELPER_DONE, "every helper ran");
    let mut rounds = Vec::new();
    for (base, count_at) in SECTIONS {
        let count = w[count_at] as usize;
        rounds.extend((0..count).map(|r| std::array::from_fn(|k| w[base + 6 * r + k])));
    }
    rounds
}

/// Runs `words` one instruction per batch on a thread with `units` set in its MSR.
///
/// Returns the PC where the thread stops, and the vector and SRR0 of the
/// interrupt it takes, if any.
fn run(words: &[u32], units: u64) -> (u64, Option<(u64, u64)>) {
    let mut mem = GuestMemory::new(0x4_0000);
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let range = ByteRange::new(GuestAddr::new(CODE), bytes.len() as u64).expect("a range");
    mem.apply_commit(range, &bytes)
        .expect("the memory holds the words");
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut()
        .set_supervisor(Some(SupervisorState::new(MSR_SF | MSR_ME | MSR_RI | units)));
    unit.state_mut().pc = CODE;
    // The loop runs one instruction per batch. The thread takes an
    // interrupt that follows a retired instruction at the start of the
    // next batch.
    for _ in words {
        let r = unit.run_until_yield(
            Budget::new(1),
            &ExecutionContext::new(&mem),
            &mut Vec::new(),
        );
        if let Some(i) = r.local_diagnostics.interrupt {
            assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
            return (unit.state().pc, Some((i.vector, i.srr0)));
        }
    }
    (unit.state().pc, None)
}

#[test]
fn a_first_fp_or_vector_add_after_a_switch_costs_an_interrupt_and_cellgov_takes_it() {
    let rounds = lazy_rounds();
    assert_eq!(rounds.len(), 48);
    for (r, d) in rounds.iter().enumerate() {
        let quiet = d[0].max(d[1]).max(d[3]).max(d[5]);
        assert!(quiet < INTERRUPT_TICKS, "round {r}: {d:?}");
        assert!(d[2] >= quiet + INTERRUPT_TICKS, "round {r}: fadd {d:?}");
        assert!(d[4] >= quiet + INTERRUPT_TICKS, "round {r}: vaddubm {d:?}");
    }
    // First use: the kernel resumed the thread with both units off. SRR0
    // names the add, which runs again once the handler returns.
    assert_eq!(run(&[ADD], 0), (CODE + 4, None), "add");
    assert_eq!(
        run(&[FADD], 0),
        (VECTOR_FP_UNAVAILABLE, Some((VECTOR_FP_UNAVAILABLE, CODE))),
        "fadd"
    );
    assert_eq!(
        run(&[VADDUBM], 0),
        (VECTOR_VXU_UNAVAILABLE, Some((VECTOR_VXU_UNAVAILABLE, CODE))),
        "vaddubm"
    );
    // Warm use: the handler enabled the unit.
    for word in [ADD, FADD, VADDUBM] {
        assert_eq!(
            run(&[word], MSR_FP | MSR_VXU),
            (CODE + 4, None),
            "{word:#010x}"
        );
    }
}

#[test]
fn dst_dss_and_the_vrsave_moves_run_with_the_vector_unit_off_as_on_the_console() {
    /// `add r9,r9,r9`, `mfspr r9,256`, `mtspr 256,r9`, `dss 0` and
    /// `dst r9,r10,0`, in the probe's candidate order.
    const CANDIDATES: [u32; 5] = [
        0x7D29_4A14,
        0x7D20_42A6,
        0x7D20_43A6,
        0x7C00_066C,
        0x7C09_52AC,
    ];
    let w = payload("ppu_lazy_unit_switch_vrsave", 672);
    assert_eq!(w[2] as usize, CANDIDATES.len());
    let rounds = w[3] as usize;
    assert!(rounds > 0);
    // A candidate is true when it runs with the unit off and leaves the
    // unit off: it costs no interrupt, and the vaddubm after it costs one.
    let console: [bool; 5] = std::array::from_fn(|c| {
        (0..rounds).all(|r| {
            let d = &w[8 + 4 * (rounds * c + r)..][..4];
            let quiet = d[0].max(d[1]).max(d[3]);
            quiet < INTERRUPT_TICKS && d[2] >= quiet + INTERRUPT_TICKS
        })
    });
    let cellgov: [bool; 5] = std::array::from_fn(|c| {
        run(&[CANDIDATES[c], VADDUBM], 0)
            == (
                VECTOR_VXU_UNAVAILABLE,
                Some((VECTOR_VXU_UNAVAILABLE, CODE + 4)),
            )
    });
    assert_eq!(console, [true; 5], "{w:?}");
    assert_eq!(cellgov, console);
}
