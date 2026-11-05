//! CellGov's PPU against the `ppu_user_msr` console capture: what the MSR
//! a PS3 user thread runs under shows to the main thread and to a created
//! thread.
//!
//! Each thread records its first FP and vector results, `addic` and `add.`
//! across the 32-bit boundary, and an `fdiv` of 0 / 0 with FPSCR VE = 1.
//! The replay runs the same instructions from a fresh thread state and
//! holds every recorded word to the capture.

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_user_msr/ps3/cech20-cex-493/observation.json";

/// The payload's word 23 when the probe wrote every word.
const DONE: u64 = 0x5050_4D53;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 192);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[23], DONE, "the probe wrote every word");
    assert_eq!(words[20], 0, "the created thread was created");
    assert_eq!(words[21], 0, "the created thread joined");
    assert_eq!(
        words[22], 0x4D53,
        "the created thread exited through its own path"
    );
    assert_eq!(
        (words[9], words[19]),
        (0x5555, 0x5556),
        "both blocks written"
    );
    words
}

const CODE: u64 = 0x1000;
const DATA: u64 = 0x2000;

/// The probe's sequence. r10 holds [`DATA`], and the data starts there:
///
/// - 1.0, 2.0, 0.0 and the sentinel at +0, +8, +16 and +24.
/// - The two vector operands at +0x40 and +0x50.
const PROGRAM: [u32; 27] = [
    0x3940_2000, // li r10, 0x2000
    0xC82A_0000, // lfd f1, 0(r10)
    0xC84A_0008, // lfd f2, 8(r10)
    0xFC21_102A, // fadd f1, f1, f2
    0x3960_0040, // li r11, 0x40
    0x7C2A_58CE, // lvx v1, r10, r11
    0x3960_0050, // li r11, 0x50
    0x7C4A_58CE, // lvx v2, r10, r11
    0x1061_1000, // vaddubm v3, v1, v2
    0x3800_0000, // li r0, 0
    0x7C01_03A6, // mtxer r0
    0x3C60_0000, // lis r3, 0
    0x6063_FFFF, // ori r3, r3, 0xFFFF
    0x6463_FFFF, // oris r3, r3, 0xFFFF
    0x3063_0001, // addic r3, r3, 1
    0x7C81_02A6, // mfxer r4
    0x3CA0_7FFF, // lis r5, 0x7FFF
    0x60A5_FFFF, // ori r5, r5, 0xFFFF
    0x3800_0001, // li r0, 1
    0x7CA5_0215, // add. r5, r5, r0
    0x7CC0_0026, // mfcr r6
    0xC88A_0010, // lfd f4, 16(r10)
    0xC86A_0018, // lfd f3, 24(r10)
    0xFF00_004C, // mtfsb1 24
    0xFC64_2024, // fdiv f3, f4, f4
    0xFCA0_048E, // mffs f5
    0x4800_0000, // b .
];

fn place(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).expect("range");
    mem.apply_commit(range, bytes).expect("mapped");
}

/// The words a fresh thread records running [`PROGRAM`], in the probe's
/// block layout (words +0 to +8).
fn cellgov_block() -> [u64; 9] {
    let mut mem = GuestMemory::new(0x4000);
    for (i, w) in PROGRAM.iter().enumerate() {
        place(&mut mem, CODE + 4 * i as u64, &w.to_be_bytes());
    }
    for (i, v) in [
        0x3FF0_0000_0000_0000u64,
        0x4000_0000_0000_0000,
        0,
        0x1234_5678_9ABC_DEF0,
    ]
    .iter()
    .enumerate()
    {
        place(&mut mem, DATA + 8 * i as u64, &v.to_be_bytes());
    }
    place(&mut mem, DATA + 0x40, &[0x01; 16]);
    place(&mut mem, DATA + 0x50, &[0x02; 16]);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = CODE;
    let end = CODE + 4 * (PROGRAM.len() as u64 - 1);
    let ctx = ExecutionContext::new(&mem);
    for _ in 0..PROGRAM.len() {
        if unit.state().pc == end {
            break;
        }
        unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    }
    let s = unit.state();
    assert_eq!(s.pc, end, "the thread ran past the fdiv");
    [
        s.fpr[1],
        (s.vr[3] >> 96) as u64,
        s.gpr[3],
        s.xer(),
        s.gpr[5],
        s.gpr[6],
        s.fpr[5] & 0xFFFF_FFFF,
        s.fpr[3],
        1,
    ]
}

#[test]
fn each_thread_sees_the_declared_user_msr_as_the_console_s_do() {
    let w = console_words();
    let c = cellgov_block();
    for (thread, base) in [("main", 0), ("created", 10)] {
        let at = |k: usize| w[base + k];
        assert_eq!(c[0], at(0), "{thread}: the first FP result");
        assert_eq!(c[1], at(1), "{thread}: the first vector result");
        assert_eq!(c[2], at(2), "{thread}: addic's result");
        assert_eq!(c[3], at(3), "{thread}: the XER after addic");
        assert_eq!(c[4], at(4), "{thread}: add.'s result");
        assert_eq!(c[5] >> 28, at(5) >> 28, "{thread}: add.'s CR0");
        assert_eq!(c[6], at(6), "{thread}: the FPSCR after the enabled 0 / 0");
        assert_eq!(c[7], at(7), "{thread}: the fdiv target");
        assert_eq!(c[8], at(8), "{thread}: the thread ran past the fdiv");
    }
}
