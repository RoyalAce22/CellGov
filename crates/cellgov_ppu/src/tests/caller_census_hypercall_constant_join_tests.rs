use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_tests::KERNEL;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The real address of the window's `nop`, after its `li r11`.
const NOP_REAL: u16 = 0x3004;

/// The real address of the window's `li r11`.
const WINDOW_REAL: u16 = 0x3000;

/// A real address with no word of the code at it.
const NOWHERE: u16 = 0x5000;

/// A word that runs only in virtual mode: no real-mode word reaches it.
const OUTSIDE: u64 = KERNEL + 0x5000;

/// The ordinals when a path enters the window's `nop` with r11 unset.
const WINDOW_UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, Some(77))];

/// `lis rt,0; ori rt,rt,value`.
fn build(rt: u8, value: u16) -> [u32; 2] {
    [
        word(PpuInstruction::Addis { rt, ra: 0, imm: 0 }),
        word(PpuInstruction::Ori {
            ra: rt,
            rs: rt,
            imm: value,
        }),
    ]
}

/// `lis rt,0x8000; rldicr rt,rt,32,31; ori rt,rt,value`, which builds
/// the kernel address `KERNEL + value`.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to 0 when RA is 0.
/// [PPC-Book1 p:72 s:3.3.12.1] `rldicr` rotates RS left by SH and ANDs it with MASK(0, ME).
fn build_kernel(rt: u8, value: u16) -> [u32; 3] {
    [
        word(PpuInstruction::Addis {
            rt,
            ra: 0,
            imm: i16::MIN,
        }),
        word(PpuInstruction::Rldicr {
            ra: rt,
            rs: rt,
            sh: 32,
            me: 31,
            rc: false,
        }),
        word(PpuInstruction::Ori {
            ra: rt,
            rs: rt,
            imm: value,
        }),
    ]
}

/// `beq` over the next `words` words.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
fn skip(words: i16) -> u32 {
    bc(0b01100, 4 * (words + 1))
}

fn addi(rt: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Addi { rt, ra, imm })
}

pub(super) fn mtctr(rs: u8) -> u32 {
    word(PpuInstruction::Mtctr { rs })
}

fn mtlr(rs: u8) -> u32 {
    word(PpuInstruction::Mtlr { rs })
}

pub(super) fn mflr(rt: u8) -> u32 {
    word(PpuInstruction::Mflr { rt })
}

pub(super) fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// `b` with the displacement `offset`.
fn b(offset: i32) -> u32 {
    word(PpuInstruction::B {
        offset,
        aa: false,
        link: false,
    })
}

/// The ordinals with the window, the second window and `words` at `at`.
pub(super) fn sites(at: u64, words: &[u32]) -> Vec<(u64, Option<u16>)> {
    let segments: [Segment; 3] = [code(at, words), window(&[]), quiet()];
    ordinals(0, &segments)
}

/// The `beq` skips the second build, so the `mtctr` also copies the
/// window's `nop`.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_gpr_that_a_branch_brings_to_its_mtctr_enters_that_constant() {
    let mut words = build(3, NOP_REAL).to_vec();
    words.push(skip(2));
    words.extend(build(3, NOWHERE));
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_UNRESOLVED);
}

/// The `beq` skips the second LR write, so the `blr` also reaches the
/// window's `nop`.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_real_mode_blr_of_an_lr_that_a_branch_brings_enters_that_constant() {
    let mut words = build(3, NOP_REAL).to_vec();
    words.extend([mtlr(3), skip(3)]);
    words.extend(build(4, NOWHERE));
    words.extend([mtlr(4), PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), WINDOW_UNRESOLVED);
}

/// The `beq` brings the window's real address to the `addi`, which steps
/// it to the window's `nop`.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_gpr_stepped_after_a_join_enters_the_step_of_each_constant() {
    let mut words = build(3, WINDOW_REAL).to_vec();
    words.push(skip(2));
    words.extend(build(3, NOWHERE));
    words.extend([addi(3, 3, 4), mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_UNRESOLVED);
}

/// The `beq` brings the LR of the first `mtlr` to the `mflr`, and the
/// `addi` steps it to the window's `nop`.
///
/// [PPC-Book1 p:82 s:3.3.13] `mflr` copies LR into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_real_mode_blr_of_an_lr_stepped_after_a_join_enters_the_step_of_each_constant() {
    let mut words = build(3, WINDOW_REAL).to_vec();
    words.extend([mtlr(3), skip(3)]);
    words.extend(build(4, NOWHERE));
    words.extend([mtlr(4), mflr(5), addi(5, 5, 4), mtlr(5), PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), WINDOW_UNRESOLVED);
}

/// The first `b` jumps over the `addi` to a build, whose `b` comes back
/// to the `addi`.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
#[test]
fn a_real_mode_bctr_of_a_gpr_that_a_later_branch_brings_enters_the_step_of_that_constant() {
    let mut words = vec![b(16), addi(3, 3, 4), mtctr(3), bctr()];
    words.extend(build(3, WINDOW_REAL));
    words.push(b(-20));
    assert_eq!(sites(VECTOR, &words), WINDOW_UNRESOLVED);
}

/// A branch in virtual mode reaches the word at the named address
/// itself, which the `beq` path steps to the window's `nop`.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_virtual_mode_bctr_of_a_gpr_stepped_after_a_join_enters_the_step_of_each_constant() {
    let mut words = build_kernel(3, WINDOW_REAL).to_vec();
    words.push(skip(3));
    words.extend(build_kernel(3, NOWHERE));
    words.extend([addi(3, 3, 4), mtctr(3), bctr()]);
    assert_eq!(sites(OUTSIDE, &words), WINDOW_UNRESOLVED);
}

/// The ordinals when an indirect branch can reach an address that the
/// scan does not know.
pub(super) const EVERY_UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, None)];

/// `lis r3,0; ori r3,r3,start; loop: addi r3,r3,0x10; blt loop`, with
/// the window's `nop` in r3 after the twentieth pass.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
fn twenty_pass_loop() -> Vec<u32> {
    let mut words = build(3, NOP_REAL - 0x10 * 20).to_vec();
    words.extend([addi(3, 3, 0x10), bc(0b01100, -4)]);
    words
}

/// The join keeps 16 of the constants of the loop in r3, so the `bctr`
/// can reach a constant that the scan dropped.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_steps_past_the_join_limit_leaves_every_site_unresolved() {
    let mut words = twenty_pass_loop();
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The join keeps 16 of the constants of the loop in r3, so the `blr`
/// can reach a constant that the scan dropped.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_of_a_gpr_that_a_loop_steps_past_the_join_limit_leaves_every_site_unresolved() {
    let mut words = twenty_pass_loop();
    words.extend([mtlr(3), PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The most constants that the join keeps in one register.
pub(super) const JOIN_LIMIT: u16 = 16;

/// The ordinals when no indirect branch reaches the window's `nop`.
pub(super) const WINDOW_RESOLVED: [(u64, Option<u16>); 2] = [(SC, Some(75)), (QUIET_SC, Some(77))];

/// `writers` builds of r3, each of its own constant, with `tail` after
/// each build and then a `beq` to the word after the last `beq`.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
fn writers(writers: u16, tail: &[u32]) -> Vec<u32> {
    let per_writer = u16::try_from(tail.len()).expect("a short tail") + 3;
    let mut words = Vec::new();
    for writer in 0..writers {
        words.extend(build(3, NOWHERE + 0x10 * writer));
        words.extend_from_slice(tail);
        let later = i16::try_from((writers - 1 - writer) * per_writer).expect("a short skip");
        words.push(skip(later));
    }
    words
}

/// `writers` `mtlr` words that each write their own constant into LR,
/// with a `beq` after each to the word after the last `beq`.
pub(super) fn lr_writers(count: u16) -> Vec<u32> {
    writers(count, &[mtlr(3)])
}

/// The join keeps 16 of the constants of the 17 writers in LR.
///
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_that_more_lr_writers_reach_than_the_join_limit_leaves_every_site_unresolved() {
    let mut words = lr_writers(JOIN_LIMIT + 1);
    words.push(PPC_BLR);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The join keeps each constant of the 16 writers in LR.
///
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_that_as_many_lr_writers_reach_as_the_join_limit_keeps_the_sites_resolved() {
    let mut words = lr_writers(JOIN_LIMIT);
    words.push(PPC_BLR);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The join keeps 16 of the constants that the 17 paths bring in r3.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_more_paths_bring_than_the_join_limit_leaves_every_site_unresolved() {
    let mut words = writers(JOIN_LIMIT + 1, &[]);
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The join keeps each constant that the 16 paths bring in r3.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_as_many_paths_bring_as_the_join_limit_keeps_the_sites_resolved() {
    let mut words = writers(JOIN_LIMIT, &[]);
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The join keeps 16 of the constants of the 17 writers in LR, so the
/// GPR that the `mflr` copies LR into can hold a constant that the scan
/// dropped.
///
/// [PPC-Book1 p:82 s:3.3.13] `mflr` copies LR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_mflr_copy_of_an_lr_past_the_join_limit_leaves_every_site_unresolved() {
    let mut words = lr_writers(JOIN_LIMIT + 1);
    words.extend([mflr(5), mtctr(5), bctr()]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}
