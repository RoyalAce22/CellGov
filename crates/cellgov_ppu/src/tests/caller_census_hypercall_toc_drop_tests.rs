use super::hypercall_constant_join_tests::{
    addi, bctr, build, descriptor, mtctr, skip, DESCRIPTORS, EVERY_UNRESOLVED, NOWHERE,
    WINDOW_RESOLVED,
};
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_join_drop_tests::RAISED_LIMIT;
use super::hypercall_tests::NOP;
use super::*;

/// Sixty-five descriptors for the word at [`VECTOR`], each with its own
/// TOC. So r2 holds each of the 65 TOCs on each word of the code, more
/// than a raised bound keeps.
fn descriptors() -> Segment {
    let bytes = (0..=RAISED_LIMIT)
        .flat_map(|toc| descriptor(VECTOR, u64::from(NOWHERE + 0x10 * toc)))
        .collect();
    data_bytes(DESCRIPTORS, bytes)
}

/// The ordinals with `words` at [`VECTOR`], the window, the second
/// window and the [`descriptors`].
fn sites(words: &[u32]) -> Vec<(u64, Option<u16>)> {
    let segments = [code(VECTOR, words), window(&[]), quiet(), descriptors()];
    ordinals(0, &segments)
}

/// `or rA,rS,rS`, which copies rS into rA.
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
fn copy(ra: u8, rs: u8) -> u32 {
    word(PpuInstruction::Or {
        ra,
        rs,
        rb: rs,
        rc: false,
    })
}

/// The first `blt` brings r2 to a join with more TOCs than the raised
/// limit. The `or` copies r2 and its mark into r5, and the second `blt`
/// brings r5 to the `mtctr`, a second join that drops the same TOCs.
///
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR, so r2 at the target is r2 at the branch.
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_copy_of_an_r2_that_joins_drop_only_tocs_from_keeps_the_sites_resolved() {
    let words = [skip(0), copy(5, 2), skip(0), mtctr(5), bctr()];
    assert_eq!(sites(&words), WINDOW_RESOLVED);
}

/// The `blt` brings r5 with the constant that the first `li` builds to
/// the `mtctr`, a join that keeps it. On the path that falls through, the
/// `or` copies r2, with that constant and each TOC, into r5. So the join
/// drops only TOCs from r5, and the `mtctr` copies the kept constant and
/// each TOC into CTR.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR, so r2 at the target is r2 at the branch.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_whose_join_keeps_a_built_constant_and_drops_only_tocs_keeps_the_sites_resolved()
{
    let built = i16::try_from(NOWHERE + 0x800).expect("a short constant");
    let words = [
        addi(5, 0, built),
        skip(2),
        addi(2, 0, built),
        copy(5, 2),
        mtctr(5),
        bctr(),
    ];
    assert_eq!(sites(&words), WINDOW_RESOLVED);
}

/// The join drops only TOCs from r2, but the `addi` adds 8 to each value
/// of r2, so r5 can hold a value that the image does not name. The
/// `mtctr` is in the interrupt vectors, where an interrupt can leave any
/// value in r5, so r5 keeps no class there.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_addi_of_an_r2_that_a_join_drops_only_tocs_from_leaves_every_site_unresolved() {
    let words = [skip(0), addi(5, 2, 8), mtctr(5), bctr()];
    assert_eq!(sites(&words), EVERY_UNRESOLVED);
}

/// The loop adds 0x10 to r2 on each pass, so its join drops values of
/// r2 that no descriptor gives. The loop starts 4 past a TOC, so a
/// dropped value and a TOC differ in the bit of value 4, and the values
/// of r2 are not a class of fewer words.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_r2_that_a_loop_steps_past_the_raised_limit_leaves_every_site_unresolved() {
    let mut words = build(2, NOWHERE + 4).to_vec();
    words.extend([addi(2, 2, 0x10), bc(0b01100, -4), mtctr(2), bctr()]);
    assert_eq!(sites(&words), EVERY_UNRESOLVED);
}

/// Each of 65 paths builds its own constant in r2, which no descriptor
/// gives, and brings it with the TOCs to the `mtctr`. The join keeps at
/// most 64 constants, so it drops a built constant with the TOCs.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to 0 when RA is 0.
/// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with UI and places the result into RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_r2_that_a_join_drops_built_constants_from_leaves_every_site_unresolved() {
    let mut words = Vec::new();
    for writer in 0..=RAISED_LIMIT {
        words.extend(build(2, NOWHERE + 0x800 + 0x10 * writer));
        // Each build takes three words with its `blt`, which goes to the
        // word after the last `blt`.
        let later = i16::try_from((RAISED_LIMIT - writer) * 3).expect("a short skip");
        words.push(skip(later));
    }
    words.extend([mtctr(2), bctr()]);
    assert_eq!(sites(&words), EVERY_UNRESOLVED);
}

/// The `or` copies r2, with each TOC, into r5. The `b` skips a word that
/// no path reaches, so it alone brings r5 to the `mtctr`, a join that
/// drops only TOCs and keeps no constant of r5. One TOC is the address
/// after the second window's `sc`. The `bdz` goes to the `bctr` on both
/// edges with that TOC less 1 in CTR, so the `bctr` can enter the `sc`
/// with r11 not set.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a01t decrements CTR, then branches if the decremented CTR M:63 is 0.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_after_a_bdz_of_a_ctr_from_a_gpr_that_a_join_drops_only_tocs_from_leaves_every_site_unresolved(
) {
    let mut bytes: Vec<u8> = (0..RAISED_LIMIT)
        .flat_map(|toc| descriptor(VECTOR, u64::from(NOWHERE + 0x10 * toc)))
        .collect();
    bytes.extend(descriptor(VECTOR, QUIET_SC + 4));
    let over = word(PpuInstruction::B {
        offset: 8,
        aa: false,
        link: false,
    });
    let words = [copy(5, 2), over, NOP, mtctr(5), bc(0b10010, 4), bctr()];
    let segments = [
        code(VECTOR, &words),
        window(&[]),
        quiet(),
        data_bytes(DESCRIPTORS, bytes),
    ];
    assert_eq!(ordinals(0, &segments), EVERY_UNRESOLVED);
}
