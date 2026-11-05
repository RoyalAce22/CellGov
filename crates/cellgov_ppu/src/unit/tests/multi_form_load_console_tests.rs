//! The `ld rT,d(r1); addi`, `lwz; or` and `or; lwz` pairs on the fused
//! path, held to the `ppu_multi_form_load_pairs` capture from the
//! reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::encode::encode;
use crate::instruction::{FusedPair, PpuInstruction as I};
use crate::state::PpuState;

/// The probe's `buf` address here, with nonzero bits on both sides of
/// bit 16 so `H | L` takes a bit from each.
const BUF: u64 = 0x1_1180;
/// `buf` with its low 16 bits cleared, and those 16 bits.
const H: u64 = BUF & !0xFFFF;
const L: u64 = BUF & 0xFFFF;
/// The probe's `buf` doublewords. The ld cases read D0 at -16(r1) and D1
/// at -8(r1), so r1 here is `BUF + 16`.
const BUF_DOUBLEWORDS: [u64; 3] = [
    0x0123_4567_89AB_CDEF,
    0x8000_0000_0000_0001,
    0xFEDC_BA98_7654_3210,
];
const SP: u64 = BUF + 16;

/// The probe's inputs A, N, B and C.
const A: u64 = 0x0123_4567_89AB_CDEF;
const N: u64 = 0x8000_0000_0000_0001;
const B: u64 = 0xF0F0_F0F0_FFFF_8001;
const C: u64 = 0x0000_0000_CAFE_F00D;

/// The CR0 field the probe sets before each recording case.
const CR0_PRESET: u8 = 0xF;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_multi_form_load_pairs/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The 64-bit register the console recorded at payload word `at`.
fn console_reg(w: &[u32], at: usize) -> u64 {
    (u64::from(w[at]) << 32) | u64::from(w[at + 1])
}

/// The CR0 field the console recorded at payload word `at`.
fn console_cr0(w: &[u32], at: usize) -> u8 {
    u8::try_from(w[at]).expect("a 4-bit CR0 field")
}

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodes")
}

/// `or rA, rS, rB`; `mr rA, rS` is `or rA, rS, rS`.
fn or(ra: u8, rs: u8, rb: u8, rc: bool) -> u32 {
    word(I::Or { ra, rs, rb, rc })
}

fn ld(rt: u8, ra: u8, imm: i16) -> u32 {
    word(I::Ld { rt, ra, imm })
}

/// `addi rT, rA, imm`; `li rT, imm` is `addi rT, 0, imm`.
fn addi(rt: u8, ra: u8, imm: i16) -> u32 {
    word(I::Addi { rt, ra, imm })
}

fn lwz(rt: u8, ra: u8, imm: i16) -> u32 {
    word(I::Lwz { rt, ra, imm })
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

fn is_ld_addi(p: &FusedPair) -> bool {
    matches!(p, FusedPair::LdAddi { .. })
}

fn is_lwz_or(p: &FusedPair) -> bool {
    matches!(p, FusedPair::LwzOr { .. })
}

fn is_or_lwz(p: &FusedPair) -> bool {
    matches!(p, FusedPair::OrLwz { .. })
}

/// Runs `steps` instructions of `words` from address 0 and returns the unit.
///
/// The start state has:
/// - the probe's `buf` doublewords at [`BUF`]
/// - r1 = [`SP`], CR0 = [`CR0_PRESET`] and XER = 0
/// - each `(register, value)` in `regs`
///
/// # Panics
///
/// Panics if:
/// - the slot at `pair` does not hold a fused pair `is_pair` accepts
/// - the run retires a count other than `steps`
fn run(
    words: &[u32],
    pair: u64,
    is_pair: fn(&FusedPair) -> bool,
    steps: u64,
    regs: &[(usize, u64)],
) -> PpuExecutionUnit {
    let code = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(shadow.get(pair), Some(I::Fused(p)) if is_pair(&p)),
        "precondition: the slot at 0x{pair:x} holds the fused pair"
    );
    let data = BUF_DOUBLEWORDS
        .iter()
        .flat_map(|d| d.to_be_bytes())
        .collect::<Vec<_>>();
    let mut mem = GuestMemory::new(0x1_2000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(BUF), data.len() as u64).unwrap(),
        &data,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    let s: &mut PpuState = unit.state_mut();
    s.set_gpr(1, SP);
    s.set_cr_field(0, CR0_PRESET);
    s.set_xer(0);
    for &(n, v) in regs {
        s.set_gpr(n, v);
    }
    unit.run_until_yield(
        Budget::new(steps),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(
        unit.retired_instructions(),
        steps,
        "the case ran to its end"
    );
    unit
}

#[test]
fn ld_from_r1_then_addi_leaves_the_registers_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 56, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let u = run(&[ld(5, 1, -16), addi(6, 5, -4)], 0, is_ld_addi, 2, &[]);
    assert_eq!(u.state().gpr[5], console_reg(&w, 2), "chained, the ld");
    assert_eq!(u.state().gpr[6], console_reg(&w, 4), "chained, the addi");

    let u = run(
        &[ld(5, 1, -8), addi(6, 0, -4)],
        0,
        is_ld_addi,
        2,
        &[(0, 0x100)],
    );
    assert_eq!(u.state().gpr[5], console_reg(&w, 6), "the ld before li");
    assert_eq!(u.state().gpr[6], console_reg(&w, 8), "li with r0 nonzero");

    let u = run(&[ld(5, 1, -16), addi(6, 1, 16)], 0, is_ld_addi, 2, &[]);
    let s = u.state();
    assert_eq!(
        s.gpr[6].wrapping_sub(s.gpr[1]),
        console_reg(&w, 10),
        "the addi reads r1"
    );

    let u = run(&[ld(5, 1, -16), addi(5, 5, -4)], 0, is_ld_addi, 2, &[]);
    assert_eq!(
        u.state().gpr[5],
        console_reg(&w, 12),
        "the addi overwrites the ld's target"
    );

    let u = run(&[ld(0, 1, -8), addi(6, 0, 1)], 0, is_ld_addi, 2, &[]);
    assert_eq!(u.state().gpr[0], console_reg(&w, 14), "the ld into r0");
    assert_eq!(
        u.state().gpr[6],
        console_reg(&w, 16),
        "an addi RA of 0 reads 0, not the loaded r0"
    );

    let u = run(
        &[B_OVER_ONE, ld(5, 1, -16), addi(6, 5, 1)],
        4,
        is_ld_addi,
        2,
        &[(5, C)],
    );
    assert_eq!(
        u.state().gpr[5],
        console_reg(&w, 52),
        "a branch over the ld"
    );
    assert_eq!(u.state().gpr[6], console_reg(&w, 54), "the addi alone");
}

#[test]
fn lwz_then_or_leaves_the_registers_and_cr0_the_console_left() {
    let w = console_words();

    let u = run(
        &[lwz(5, 7, 0), or(6, 5, 4, true)],
        0,
        is_lwz_or,
        2,
        &[(7, BUF), (4, B)],
    );
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 18), "chained, the lwz");
    assert_eq!(s.gpr[6], console_reg(&w, 20), "chained, the or.");
    assert_eq!(s.cr_field(0), console_cr0(&w, 22), "the or. records");

    let u = run(
        &[lwz(5, 7, 4), or(6, 5, 5, false)],
        0,
        is_lwz_or,
        2,
        &[(7, BUF)],
    );
    assert_eq!(u.state().gpr[5], console_reg(&w, 23), "the lwz before mr");
    assert_eq!(
        u.state().gpr[6],
        console_reg(&w, 25),
        "mr of the loaded word"
    );

    let u = run(
        &[lwz(7, 7, 8), or(6, 7, 4, false)],
        0,
        is_lwz_or,
        2,
        &[(7, BUF), (4, C)],
    );
    assert_eq!(
        u.state().gpr[7],
        console_reg(&w, 27),
        "the lwz into its base"
    );
    assert_eq!(
        u.state().gpr[6],
        console_reg(&w, 29),
        "the or reads the new r7"
    );

    let u = run(
        &[B_OVER_ONE, lwz(5, 7, 0), or(6, 5, 4, false)],
        4,
        is_lwz_or,
        2,
        &[(7, BUF), (4, A), (5, C)],
    );
    assert_eq!(
        u.state().gpr[5],
        console_reg(&w, 48),
        "a branch over the lwz"
    );
    assert_eq!(u.state().gpr[6], console_reg(&w, 50), "the or alone");
}

#[test]
fn or_then_lwz_loads_from_the_address_the_console_loaded() {
    let w = console_words();

    let u = run(
        &[or(5, 8, 9, false), lwz(6, 5, 4)],
        0,
        is_or_lwz,
        2,
        &[(8, H), (9, L)],
    );
    assert_eq!(
        u.state().gpr[5].wrapping_sub(BUF),
        console_reg(&w, 31),
        "the or builds buf"
    );
    assert_eq!(
        u.state().gpr[6],
        console_reg(&w, 33),
        "the lwz reads through it"
    );

    let u = run(
        &[or(5, 4, 3, true), lwz(6, 7, 8)],
        0,
        is_or_lwz,
        2,
        &[(3, A), (4, N), (7, BUF)],
    );
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 35), "the or.");
    assert_eq!(s.gpr[6], console_reg(&w, 37), "the lwz from another base");
    assert_eq!(s.cr_field(0), console_cr0(&w, 39), "the or. records");

    let u = run(
        &[or(7, 7, 4, false), lwz(6, 7, 12)],
        0,
        is_or_lwz,
        2,
        &[(7, H), (4, L)],
    );
    assert_eq!(
        u.state().gpr[7].wrapping_sub(BUF),
        console_reg(&w, 40),
        "or 7,7,4 builds buf"
    );
    assert_eq!(
        u.state().gpr[6],
        console_reg(&w, 42),
        "the lwz on the new r7"
    );

    let u = run(
        &[or(6, 5, 5, false), lwz(5, 6, 16)],
        0,
        is_or_lwz,
        2,
        &[(5, BUF)],
    );
    assert_eq!(
        u.state().gpr[6].wrapping_sub(BUF),
        console_reg(&w, 44),
        "mr 6,5"
    );
    assert_eq!(
        u.state().gpr[5],
        console_reg(&w, 46),
        "the lwz writes the or's source"
    );
}
