//! CellGov's `lswi` and `stswi` against the `ppu_string_word_slots`
//! console capture: the byte order in each register, a short last
//! register, the wrap from r31 to r0, the zeroed high-order word of each
//! `lswi` target, and a byte count of 0.
//!
//! Each case starts with 0x8000000000008000 in r0 and the buffer address
//! in r7, with byte i of the 64-byte buffer at 0xC0 + i. The words are
//! the ones the probe's ELF holds.

use cellgov_ppu::differential::{execute_state_in, PpuStateSnapshot};

/// The probe's done marker, "STRW".
const DONE: u64 = 0x5354_5257;

/// Where the replay puts the buffer.
const BASE: u64 = 0x1_0000;

/// The GPR that holds the buffer address in the probe's ELF.
const B: usize = 7;

/// The r0 that each case starts with.
const R0: u64 = 0x8000_0000_0000_8000;

/// The probe's payload, as big-endian u64 words.
fn console() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_string_word_slots/ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * 24, "payload bytes");
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[23], DONE, "the probe wrote every word");
    words
}

/// CellGov's GPRs and buffer after `words`, from r0 = [`R0`], the
/// buffer address in r7 and each `(gpr, value)` of `set`.
fn run(words: &[u32], set: &[(usize, u64)]) -> (PpuStateSnapshot, Vec<u8>) {
    let mut s = PpuStateSnapshot::zero();
    s.gpr[0] = R0;
    s.gpr[B] = BASE;
    for &(gpr, value) in set {
        s.gpr[gpr] = value;
    }
    let mut mem: Vec<u8> = (0..64u8).map(|i| 0xC0 + i).collect();
    for &w in words {
        (s, mem) =
            execute_state_in(w, &s, BASE, mem).unwrap_or_else(|e| panic!("{w:#010x}: {e:?}"));
    }
    (s, mem)
}

/// Buffer doubleword `n`.
fn dword(mem: &[u8], n: usize) -> u64 {
    u64::from_be_bytes(mem[8 * n..8 * n + 8].try_into().expect("8 bytes"))
}

/// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of RS onward, left to right, to EA = (RA|0).
#[test]
fn a_string_store_writes_the_bytes_and_the_load_back_the_console_records() {
    let w = console();
    // stswi r0,r7,4
    let (_, mem) = run(&[0x7C07_25AA], &[]);
    assert_eq!(dword(&mem, 0), w[0], "A: the buffer");
    // stswi r0,r7,4; lwz r0,0(r7)
    assert_eq!(
        run(&[0x7C07_25AA, 0x8007_0000], &[]).0.gpr[0],
        w[1],
        "A: r0"
    );
    // stswi r0,r7,2
    let (_, mem) = run(&[0x7C07_15AA], &[]);
    assert_eq!(dword(&mem, 0), w[2], "B: the buffer");
    // stswi r0,r7,2; lhz r0,0(r7)
    assert_eq!(
        run(&[0x7C07_15AA, 0xA007_0000], &[]).0.gpr[0],
        w[3],
        "B: r0"
    );
    // stswi r31,r7,8; lwz r0,4(r7)
    let r31 = [(31, 0x1111_1111_2233_4455)];
    let (s, mem) = run(&[0x7FE7_45AA, 0x8007_0004], &r31);
    assert_eq!((dword(&mem, 0), s.gpr[0]), (w[15], w[16]), "F");
    // stswi r25,r7,0; lwz r0,28(r7)
    let sources: Vec<(usize, u64)> = (0..7)
        .map(|k| (25 + k, 0xF0F0_F0F0_A0B0_C0D0 + k as u64))
        .collect();
    let (s, mem) = run(&[0x7F27_05AA, 0x8007_001C], &sources);
    let stored: Vec<u64> = (0..5).map(|n| dword(&mem, n)).collect();
    assert_eq!(stored, w[17..22], "G: the buffer");
    assert_eq!(s.gpr[0], w[22], "G: r0");
}

/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads n bytes from EA = (RA|0) into the low-order words of RT onward, left to right, with GPR 0 after GPR 31, and sets the high-order word and the unfilled bytes to 0.
#[test]
fn a_string_load_fills_the_registers_the_console_records() {
    let w = console();
    // std r0,0(r7); lswi r31,r7,8
    let s = run(&[0xF807_0000, 0x7FE7_44AA], &[]).0;
    assert_eq!([s.gpr[31], s.gpr[0]], w[4..6], "C");
    // std r0,24(r7); lswi r25,r7,0
    let s = run(&[0xF807_0018, 0x7F27_04AA], &[]).0;
    assert_eq!(s.gpr[25..32], w[6..13], "D: r25..r31");
    assert_eq!(s.gpr[0], w[13], "D: r0");
    // std r0,0(r7); lswi r0,r7,2
    assert_eq!(run(&[0xF807_0000, 0x7C07_14AA], &[]).0.gpr[0], w[14], "E");
}
