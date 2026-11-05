//! The `rldicr; oris`, `or; or`, `or; bc` and `or; bctrl` pairs on the
//! fused path, held to the `ppu_multi_form_reg_pairs` capture from the
//! reference console.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::encode::encode;
use crate::instruction::{FusedPair, PpuInstruction as I};
use crate::state::PpuState;

/// The probe's inputs A, N, B and C.
const A: u64 = 0x0123_4567_89AB_CDEF;
const N: u64 = 0x8000_0000_0000_0001;
const B: u64 = 0xF0F0_F0F0_FFFF_8001;
const C: u64 = 0x0000_0000_CAFE_F00D;

/// The CR0 field the probe sets before each recording case.
const CR0_PRESET: u8 = 0xF;
/// The CR the probe sets before its two beq cases on a preset CR.
const CR_ALL_EQ: u32 = 0x2222_2222;
/// The XER the probe sets before its XER[SO] case.
const XER_SO: u64 = 1 << 31;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_multi_form_reg_pairs/ps3/cech20-cex-493/cgov_frame.bin");
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

/// `rldicr rA, rS, sh, me`; `sldi rA, rS, n` is `rldicr rA, rS, n, 63-n`.
fn rldicr(ra: u8, rs: u8, sh: u8, me: u8, rc: bool) -> u32 {
    word(I::Rldicr { ra, rs, sh, me, rc })
}

fn oris(ra: u8, rs: u8, imm: u16) -> u32 {
    word(I::Oris { ra, rs, imm })
}

/// A relative, non-linking `bc` whose target is `offset` bytes from it.
fn bc(bo: u8, bi: u8, offset: i16) -> u32 {
    word(I::Bc {
        bo,
        bi,
        offset,
        aa: false,
        link: false,
    })
}

fn li(rt: u8, imm: i16) -> u32 {
    word(I::Addi { rt, ra: 0, imm })
}

fn bctrl() -> u32 {
    word(I::Bcctr {
        bo: 20,
        bi: 0,
        link: true,
    })
}

const B_OVER_ONE: u32 = (18 << 26) | 8;

fn is_rldicr_oris(p: &FusedPair) -> bool {
    matches!(p, FusedPair::RldicrOris { .. })
}

fn is_or_or(p: &FusedPair) -> bool {
    matches!(p, FusedPair::OrOr { .. })
}

fn is_or_bc(p: &FusedPair) -> bool {
    matches!(p, FusedPair::OrBc { .. })
}

fn is_or_bctrl(p: &FusedPair) -> bool {
    matches!(p, FusedPair::OrBctrl { .. })
}

/// Runs `steps` instructions of `words` from address 0, after `init`
/// sets the start state, and returns the unit.
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
    init: impl FnOnce(&mut PpuState),
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
    let mut mem = GuestMemory::new(0x100);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    init(unit.state_mut());
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

/// The start state of a recording case: CR0 = [`CR0_PRESET`], XER = `xer`
/// and GPRs `regs` as `(register, value)`.
fn preset(regs: &[(usize, u64)], xer: u64) -> impl FnOnce(&mut PpuState) + '_ {
    move |s| {
        s.set_cr_field(0, CR0_PRESET);
        s.set_xer(xer);
        for &(n, v) in regs {
            s.set_gpr(n, v);
        }
    }
}

#[test]
fn rldicr_then_oris_leaves_the_registers_and_cr0_the_console_left() {
    let w = console_words();
    assert_eq!(w.len(), 74, "payload words");
    assert_eq!(w[0], 0, "the probe ran to its end");

    let code = [rldicr(5, 4, 8, 40, true), oris(6, 5, 0x8001)];
    let u = run(&code, 0, is_rldicr_oris, 2, preset(&[(4, A)], 0));
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 2), "chained, the rldicr.");
    assert_eq!(s.gpr[6], console_reg(&w, 4), "chained, the oris");
    assert_eq!(s.cr_field(0), console_cr0(&w, 6), "chained, CR0");

    let code = [rldicr(5, 4, 4, 59, false), oris(6, 5, 0x8001)];
    let u = run(&code, 0, is_rldicr_oris, 2, preset(&[(4, A)], 0));
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 7), "sldi");
    assert_eq!(s.gpr[6], console_reg(&w, 9), "oris after sldi");
    assert_eq!(s.cr_field(0), console_cr0(&w, 11), "sldi does not record");

    let code = [rldicr(5, 4, 0, 63, true), oris(4, 4, 0xFFFF)];
    let u = run(&code, 0, is_rldicr_oris, 2, preset(&[(4, N)], 0));
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 12), "rldicr. of a negative");
    assert_eq!(s.gpr[4], console_reg(&w, 14), "oris writes its source");
    assert_eq!(s.cr_field(0), console_cr0(&w, 16), "a negative result");

    let code = [rldicr(5, 4, 0, 0, true), oris(6, 5, 0)];
    let u = run(&code, 0, is_rldicr_oris, 2, preset(&[(4, A)], XER_SO));
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 17), "a zero rldicr.");
    assert_eq!(s.gpr[6], console_reg(&w, 19), "oris of zero");
    assert_eq!(s.cr_field(0), console_cr0(&w, 21), "zero under XER[SO]");
}

#[test]
fn or_then_or_leaves_the_registers_and_cr0_the_console_left() {
    let w = console_words();

    let code = [or(5, 4, 3, false), or(6, 5, 3, true)];
    let u = run(&code, 0, is_or_or, 2, preset(&[(3, A), (4, B)], 0));
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 22), "chained, the first or");
    assert_eq!(s.gpr[6], console_reg(&w, 24), "chained, the or.");
    assert_eq!(s.cr_field(0), console_cr0(&w, 26), "the second records");

    let code = [or(5, 4, 4, false), or(6, 5, 5, false)];
    let u = run(&code, 0, is_or_or, 2, |s| s.set_gpr(4, B));
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 27), "mr then mr, the first");
    assert_eq!(s.gpr[6], console_reg(&w, 29), "mr then mr, the second");

    let code = [or(5, 4, 3, true), or(6, 7, 7, true)];
    let u = run(&code, 0, is_or_or, 2, preset(&[(3, A), (4, N), (7, 0)], 0));
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 31), "both record, the first");
    assert_eq!(s.gpr[6], console_reg(&w, 33), "both record, the second");
    assert_eq!(s.cr_field(0), console_cr0(&w, 35), "the second's CR0 wins");

    let code = [or(4, 4, 3, true), or(3, 4, 4, false)];
    let u = run(&code, 0, is_or_or, 2, preset(&[(3, A), (4, B)], 0));
    let s = u.state();
    assert_eq!(s.gpr[4], console_reg(&w, 36), "or. 4,4,3");
    assert_eq!(
        s.gpr[3],
        console_reg(&w, 38),
        "the second writes the first's source"
    );
    assert_eq!(s.cr_field(0), console_cr0(&w, 40), "the first records");

    let code = [B_OVER_ONE, or(5, 4, 3, false), or(6, 5, 3, false)];
    let u = run(&code, 4, is_or_or, 2, |s| {
        s.set_gpr(3, A);
        s.set_gpr(4, B);
        s.set_gpr(5, C);
    });
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 59), "a branch over the first or");
    assert_eq!(s.gpr[6], console_reg(&w, 61), "the second or alone");
}

/// CR bit 2 (CR0 EQ), which `beq` tests.
const BI_EQ: u8 = 2;
/// CR bit 0 (CR0 LT), which `blt` tests.
const BI_LT: u8 = 0;

/// Runs `or_word ; bc 12,bi,+8 ; li 7,1 ; li 8,1` for three instructions,
/// so r7 is 1 exactly when the bc falls through.
fn or_bc_case(or_word: u32, bi: u8, init: impl FnOnce(&mut PpuState)) -> PpuExecutionUnit {
    let code = [or_word, bc(12, bi, 8), li(7, 1), li(8, 1)];
    run(&code, 0, is_or_bc, 3, init)
}

/// The start state of the beq cases on a preset CR.
fn all_eq(s: &mut PpuState) {
    s.set_cr(CR_ALL_EQ);
    s.set_xer(0);
    s.set_gpr(3, A);
    s.set_gpr(4, B);
    s.set_gpr(7, 0);
}

#[test]
fn or_then_bc_branches_as_the_console_branched() {
    let w = console_words();

    let u = or_bc_case(
        or(5, 4, 3, true),
        BI_EQ,
        preset(&[(3, 0), (4, 0), (7, 0)], 0),
    );
    assert_eq!(
        u.state().gpr[7],
        console_reg(&w, 41),
        "a recorded zero, beq"
    );
    assert_eq!(u.state().cr_field(0), console_cr0(&w, 43), "CR0 of zero");

    let u = or_bc_case(
        or(5, 4, 3, true),
        BI_LT,
        preset(&[(3, A), (4, N), (7, 0)], 0),
    );
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 44), "a recorded negative");
    assert_eq!(s.gpr[7], console_reg(&w, 46), "blt on that CR0");
    assert_eq!(s.cr_field(0), console_cr0(&w, 48), "CR0 of a negative");

    let u = or_bc_case(or(5, 4, 3, false), BI_EQ, all_eq);
    assert_eq!(
        u.state().gpr[7],
        console_reg(&w, 49),
        "beq on the preset CR0"
    );
    assert_eq!(u.state().cr(), w[51], "a non-recording or leaves the CR");

    let u = or_bc_case(or(5, 4, 3, true), BI_EQ, all_eq);
    assert_eq!(
        u.state().gpr[7],
        console_reg(&w, 52),
        "beq on the or.'s CR0"
    );
    assert_eq!(u.state().cr(), w[54], "the or. writes CR0 alone");
}

/// Runs `or 5,4,3 ; 0: bdnz 0b` with CTR = 3. The bdnz runs three times:
/// once inside the pair, then twice alone.
#[test]
fn or_then_bdnz_to_its_own_address_counts_as_the_console_counted() {
    let w = console_words();
    let code = [or(5, 4, 3, false), bc(16, 0, 0)];
    let u = run(&code, 0, is_or_bc, 4, |s| {
        s.set_gpr(3, A);
        s.set_gpr(4, B);
        s.set_ctr(3);
    });
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 55), "the or");
    assert_eq!(s.ctr(), console_reg(&w, 57), "CTR after the loop");
    assert_eq!(s.pc, 8, "the loop fell through");
}

/// Runs `or ; bctrl ; li 7,1 ; T: li 8,1` for three instructions, with
/// CTR = T or T + 3. The probe records LR minus CTR at T.
#[test]
fn or_then_bctrl_links_and_branches_as_the_console_did() {
    let w = console_words();
    let t = 12;

    let code = [or(5, 4, 3, true), bctrl(), li(7, 1), li(8, 1)];
    let u = run(&code, 0, is_or_bctrl, 3, |s| {
        preset(&[(3, A), (4, B), (7, 0)], 0)(s);
        s.set_ctr(t);
    });
    let s = u.state();
    assert_eq!(s.gpr[5], console_reg(&w, 63), "the or.");
    assert_eq!(s.gpr[7], console_reg(&w, 65), "the bctrl skips li 7,1");
    assert_eq!(
        s.lr().wrapping_sub(s.ctr()),
        console_reg(&w, 67),
        "LR is the address after the bctrl"
    );
    assert_eq!(s.cr_field(0), console_cr0(&w, 69), "the or. records");

    let u = run(&code, 0, is_or_bctrl, 3, |s| {
        s.set_gpr(3, A);
        s.set_gpr(4, B);
        s.set_gpr(7, 0);
        s.set_ctr(t + 3);
    });
    let s = u.state();
    assert_eq!(s.gpr[7], console_reg(&w, 70), "CTR's low bits are ignored");
    assert_eq!(
        s.lr().wrapping_sub(s.ctr()),
        console_reg(&w, 72),
        "LR minus a CTR with its low bits set"
    );
}
