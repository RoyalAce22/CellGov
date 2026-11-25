use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_join_drop_tests::RAISED_LIMIT;
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

/// `blt` over the next `words` words: a `bc` on CR bit 0.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
fn skip(words: i16) -> u32 {
    bc(0b01100, 4 * (words + 1))
}

/// The targets that the bound gives the word `writer` of the code
/// `words` at `at`, where `writer` writes CTR or LR.
///
/// The image names each constant that a word builds. So the census
/// enters a constant that a branch brings unchanged also with no join.
/// The targets of the writer hold the constant only when the join
/// brings it.
fn targets_of(at: u64, words: &[u32], writer: u32) -> BTreeSet<u64> {
    let index = words
        .iter()
        .position(|&raw| raw == writer)
        .expect("the code holds the writer");
    let code = Code::new(vec![CodeSegment {
        vaddr: at,
        words: words.to_vec(),
    }]);
    let mut bound = indirect_entries(&[], &[], &code).expect("the code holds no jump table");
    bound
        .targets
        .remove(&(at + 4 * index as u64))
        .unwrap_or_default()
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

/// The `blt` skips the second build, so the `mtctr` also copies the
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
    assert_eq!(
        targets_of(VECTOR, &words, mtctr(3)),
        BTreeSet::from([NOP_REAL, NOWHERE].map(u64::from))
    );
    assert_eq!(sites(VECTOR, &words), WINDOW_UNRESOLVED);
}

/// The `blt` skips the second LR write, so the `blr` also reaches the
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
    assert_eq!(
        targets_of(VECTOR, &words, mtlr(3)),
        BTreeSet::from([u64::from(NOP_REAL)])
    );
    assert_eq!(sites(VECTOR, &words), WINDOW_UNRESOLVED);
}

/// The `blt` brings the window's real address to the `addi`, which steps
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

/// The `blt` brings the LR of the first `mtlr` to the `mflr`, and the
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

/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
#[test]
fn a_real_mode_bctr_of_a_gpr_that_a_later_branch_brings_enters_the_step_of_that_constant() {
    let mut words = vec![b(16), addi(3, 3, 4), mtctr(3), bctr()];
    words.extend(build(3, WINDOW_REAL));
    words.push(b(-20));
    assert_eq!(sites(VECTOR, &words), WINDOW_UNRESOLVED);
}

/// A branch in virtual mode reaches the word at the named address
/// itself, which the `blt` path steps to the window's `nop`.
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
pub(super) fn twenty_pass_loop() -> Vec<u32> {
    let mut words = build(3, NOP_REAL - 0x10 * 20).to_vec();
    words.extend([addi(3, 3, 0x10), bc(0b01100, -4)]);
    words
}

/// The loop brings more constants in r3 to its join than any bound, so
/// the `bctr` can reach a constant that the scan dropped.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_steps_past_the_join_limit_leaves_every_site_unresolved() {
    let mut words = twenty_pass_loop();
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The loop brings more constants in r3 to its join than any bound, so
/// the `blr` can reach a constant that the scan dropped.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_of_a_gpr_that_a_loop_steps_past_the_join_limit_leaves_every_site_unresolved() {
    let mut words = twenty_pass_loop();
    words.extend([mtlr(3), PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The loop marks r3 at its own join. The `blt` after the loop brings
/// the kept constants and the mark to the `mtctr`, a second join. When
/// the bound of the second join rises, it does not overflow and keeps
/// the mark.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_marks_lost_before_a_later_join_leaves_every_site_unresolved() {
    let mut words = twenty_pass_loop();
    words.extend([skip(0), mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The loop marks r3 at its own join, and the `mtlr` copies the mark
/// into LR. The `blt` after it brings the kept constants and the mark
/// to the `blr`, a second join. When the bound of the second join
/// rises, it does not overflow and keeps the mark.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_of_an_lr_that_a_loop_marks_lost_before_a_later_join_leaves_every_site_unresolved() {
    let mut words = twenty_pass_loop();
    words.extend([mtlr(3), skip(0), PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The loop marks r3, and the build after it writes r3 from 0. So the
/// `mtctr` copies only the constant of the build.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to 0 when RA is 0.
/// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with UI and places the result into RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_build_rewrites_after_a_loop_marks_it_lost_keeps_the_sites_resolved() {
    let mut words = twenty_pass_loop();
    words.extend(build(3, NOWHERE));
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The most constants that the join keeps in one register.
pub(super) const JOIN_LIMIT: u16 = 16;

/// The ordinals when no indirect branch reaches the window's `nop`.
pub(super) const WINDOW_RESOLVED: [(u64, Option<u16>); 2] = [(SC, Some(75)), (QUIET_SC, Some(77))];

/// `writers` builds of r3, each of its own constant, with `tail` after
/// each build and then a `blt` to the word after the last `blt`.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
pub(super) fn writers(writers: u16, tail: &[u32]) -> Vec<u32> {
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

/// `count` `mtlr` words that each write their own constant into LR,
/// with a `blt` after each to the word after the last `blt`.
pub(super) fn lr_writers(count: u16) -> Vec<u32> {
    writers(count, &[mtlr(3)])
}

/// The 65 writers bring more constants in LR than a raised bound keeps,
/// so the bound of the join does not rise, and the join keeps 16 of
/// them.
///
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_that_more_lr_writers_reach_than_the_raised_limit_leaves_every_site_unresolved() {
    let mut words = lr_writers(RAISED_LIMIT + 1);
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

/// Fifteen LR writers reach the `blr`, and one of them, the `mtlr r4`,
/// brings one constant and then a second. With the second constant LR
/// holds 16 constants, as many as the join limit, so the join takes it.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_join_takes_a_second_constant_of_one_lr_writer_that_fills_the_lr_to_the_join_limit() {
    let first = NOWHERE + 0x10 * JOIN_LIMIT;
    let second = first + 0x10;
    let mut words = lr_writers(JOIN_LIMIT - 2);
    words.push(PPC_BLR);
    words.extend(build(4, first));
    // Each `b` goes back four words: to the `blr`, then to the `mtlr`.
    words.extend([mtlr(4), b(-16)]);
    words.extend(build(4, second));
    words.push(b(-16));
    assert_eq!(
        targets_of(VECTOR, &words, mtlr(4)),
        BTreeSet::from([first, second].map(u64::from))
    );
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The address of the data segment that holds the descriptors.
const DESCRIPTORS: u64 = KERNEL + 0x2_0000;

/// The bytes of a function descriptor for the word at `entry` with the
/// TOC `toc`.
///
/// [OFW-4.93 lv2_kernel a:0x80000000002765e4-0x8000000000276600] A call through a descriptor loads LR from its first doubleword and r2 from its second.
fn descriptor(entry: u64, toc: u64) -> Vec<u8> {
    [entry, toc, 0]
        .iter()
        .flat_map(|doubleword| doubleword.to_be_bytes())
        .collect()
}

/// Sixty-five descriptors give the first word sixty-five TOCs, so r2
/// holds each of them on the path to the `mtctr`. The `blt` brings that
/// r2 to the `mtctr`, a join with more TOCs than the raised limit, so
/// the join marks r2.
///
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR, so r2 at the target is r2 at the branch.
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_r2_with_more_tocs_than_the_raised_limit_at_a_join_leaves_every_site_unresolved() {
    let words = [skip(0), mtctr(2), bctr()];
    let descriptors: Vec<u8> = (0..=RAISED_LIMIT)
        .flat_map(|toc| descriptor(VECTOR, u64::from(NOWHERE + 0x10 * toc)))
        .collect();
    let segments = [
        code(VECTOR, &words),
        window(&[]),
        quiet(),
        data_bytes(DESCRIPTORS, descriptors),
    ];
    assert_eq!(ordinals(0, &segments), EVERY_UNRESOLVED);
}

/// The 65 paths bring more constants in r3 than a raised bound keeps,
/// so the bound of the join does not rise, and the join keeps 16 of
/// them.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_more_paths_bring_than_the_raised_limit_leaves_every_site_unresolved() {
    let mut words = writers(RAISED_LIMIT + 1, &[]);
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

/// The 16 paths join at the last `blt`, which brings each of their
/// constants in r3 to the `mtctr` on one path. The `mtctr` is a second
/// join that gets the 16 constants first, and keeps each of them.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_one_path_brings_at_the_join_limit_keeps_the_sites_resolved() {
    let mut words = writers(JOIN_LIMIT, &[]);
    words.extend([skip(0), mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The 65 writers bring more constants in LR than a raised bound keeps,
/// so the join drops some of them, and the GPR that the `mflr` copies
/// LR into can hold a constant that the scan dropped.
///
/// [PPC-Book1 p:82 s:3.3.13] `mflr` copies LR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_mflr_copy_of_an_lr_past_the_raised_limit_leaves_every_site_unresolved() {
    let mut words = lr_writers(RAISED_LIMIT + 1);
    words.extend([mflr(5), mtctr(5), bctr()]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}
