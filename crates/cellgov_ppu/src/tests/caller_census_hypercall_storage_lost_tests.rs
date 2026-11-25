use super::hypercall_constant_join_tests::{
    bctr, mtctr, sites, twenty_pass_loop, writers, EVERY_UNRESOLVED, JOIN_LIMIT, WINDOW_RESOLVED,
};
use super::hypercall_ctr_decrement_tests::{quiet, window, VECTOR};
use super::hypercall_entry_tests::{code, elf, word};
use super::*;

/// The offset from r1 of the stack slot that the tests store to and load
/// from.
const SLOT: i16 = -8;

/// `std rs,imm(ra)`.
fn std(rs: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Std { rs, ra, imm })
}

/// `ld rt,imm(ra)`.
fn ld(rt: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Ld { rt, ra, imm })
}

/// The loop that marks r3 lost, then `store`, then `tail`.
fn stored_after_loop(store: u32, tail: &[u32]) -> Vec<u32> {
    let mut words = twenty_pass_loop();
    words.push(store);
    words.extend_from_slice(tail);
    words
}

/// `load`, then `mtctr r4; bctr`.
fn bctr_of_r4_after(load: u32) -> [u32; 3] {
    [load, mtctr(4), bctr()]
}

/// What [`hypercall_losses`] gives for the window, the second window and
/// `words` at [`VECTOR`].
fn losses(words: &[u32]) -> HypercallLosses {
    let segments = [code(VECTOR, words), window(&[]), quiet()];
    hypercall_losses(&elf(0, &segments)).expect("the ELF scans")
}

/// The `std` stores r5 through the lost r3. r5 holds no dropped
/// constant, so the `ld` loads none. A `bctr` of r3 after the `std`
/// leaves every site unresolved, so r3 is lost at the `ld`.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_load_after_a_store_through_a_lost_base_keeps_the_sites_resolved() {
    let store = std(5, 3, 0);
    let base = stored_after_loop(store, &[mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &base), EVERY_UNRESOLVED);
    let words = stored_after_loop(store, &bctr_of_r4_after(ld(4, 3, 0)));
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The loop marks r3 lost, but the code stores only r5, which holds the
/// constant that `lis` and `ori` build. A `bctr` of r3 after the `std`
/// leaves every site unresolved, so r3 is lost at the `ld`.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to 0 when RA is 0.
/// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with UI and places the result into RA.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_load_after_a_store_of_a_built_constant_keeps_the_sites_resolved() {
    let build = [
        word(PpuInstruction::Addis {
            rt: 5,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 5,
            rs: 5,
            imm: 0x5000,
        }),
    ];
    let stored = |tail: &[u32]| {
        let mut words = twenty_pass_loop();
        words.extend(build);
        words.push(std(5, 1, SLOT));
        words.extend_from_slice(tail);
        words
    };
    let base = stored(&[mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &base), EVERY_UNRESOLVED);
    let words = stored(&bctr_of_r4_after(ld(4, 1, SLOT)));
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The 17 paths bring one constant in r3 more than the join keeps, and
/// the `std` stores the lost r3. The census carries no mark through
/// storage, so the `ld` gives r4 no mark and no word takes the drop.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_load_of_a_gpr_stored_after_a_join_past_its_limit_keeps_the_sites_resolved() {
    let mut words = writers(JOIN_LIMIT + 1, &[]);
    words.push(std(3, 1, SLOT));
    words.extend(bctr_of_r4_after(ld(4, 1, SLOT)));
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
    assert_eq!(losses(&words), HypercallLosses::default());
}
