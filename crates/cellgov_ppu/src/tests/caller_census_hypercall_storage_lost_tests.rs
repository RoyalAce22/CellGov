use super::hypercall_constant_join_tests::{
    bctr, mtctr, sites, twenty_pass_loop, writers, EVERY_UNRESOLVED, JOIN_LIMIT, WINDOW_RESOLVED,
};
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, VECTOR};
use super::hypercall_entry_tests::{code, elf, word};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

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

fn mtlr(rs: u8) -> u32 {
    word(PpuInstruction::Mtlr { rs })
}

/// `blt` to the word `words` words after the next word.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
fn blt_over(words: i16) -> u32 {
    bc(0b01100, 4 * (words + 1))
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

/// The address of the word at `index` of the code at [`VECTOR`].
fn at(index: usize) -> u64 {
    VECTOR + 4 * index as u64
}

/// The loop leaves the window's `nop` in r3 after its twentieth pass,
/// and the join drops that constant. The `ld` reads it back into r4.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
#[ignore = "the census carries no lost mark through storage yet, and the sound rule unresolves every held kernel"]
fn a_bctr_of_a_lost_gpr_that_a_store_and_a_load_move_through_the_stack_leaves_every_site_unresolved(
) {
    let words = stored_after_loop(std(3, 1, SLOT), &bctr_of_r4_after(ld(4, 1, SLOT)));
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
#[ignore = "the census carries no lost mark through storage yet, and the sound rule unresolves every held kernel"]
fn a_blr_of_a_lost_gpr_that_a_store_and_a_load_move_through_the_stack_leaves_every_site_unresolved()
{
    let words = stored_after_loop(std(3, 1, SLOT), &[ld(4, 1, SLOT), mtlr(4), PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The `blt` at word 0 takes one path to the loop and its store of r3
/// through r9. The other path runs the `ld` and never reaches the store.
/// Storage outlives the path, so the `ld` can still read the stored value.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
#[ignore = "the census carries no lost mark through storage yet, and the sound rule unresolves every held kernel"]
fn a_bctr_of_a_load_on_a_path_that_misses_the_store_of_a_lost_gpr_leaves_every_site_unresolved() {
    let mut words = vec![blt_over(3)];
    words.extend(bctr_of_r4_after(ld(4, 9, 0)));
    words.extend(stored_after_loop(std(3, 9, 0), &[PPC_BLR]));
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// As in the test above, but the `blt` at word 2 makes the `mtctr` at
/// word 3 a join word. The flow reaches that join from the `ld` before it
/// steps the store, so the mark of r4 must reach the join on a later
/// pass.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
#[ignore = "the census carries no lost mark through storage yet, and the sound rule unresolves every held kernel"]
fn a_bctr_at_a_join_after_a_load_that_the_flow_steps_before_the_store_leaves_every_site_unresolved()
{
    let mut words = vec![blt_over(4), ld(4, 9, 0), blt_over(0), mtctr(4), bctr()];
    words.extend(stored_after_loop(std(3, 9, 0), &[PPC_BLR]));
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The `std` stores r5 through the lost r3. r5 holds no dropped
/// constant, so the `ld` loads none.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_load_after_a_store_through_a_lost_base_keeps_the_sites_resolved() {
    let words = stored_after_loop(std(5, 3, 0), &bctr_of_r4_after(ld(4, 3, 0)));
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The loop marks r3 lost, but the code stores only r5, which holds the
/// constant that `lis` and `ori` build.
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
    let mut words = twenty_pass_loop();
    words.extend(build);
    words.push(std(5, 1, SLOT));
    words.extend(bctr_of_r4_after(ld(4, 1, SLOT)));
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// Each store, which stores r3 or a run of GPRs from `rs`, and whether
/// it stores r3.
///
/// [PPC-Book1 p:42 s:3.3.3] `stw` stores RS 32:63 into the word at EA.
/// [PPC-Book1 p:45 s:3.3.4] `sthbrx` stores the low-order halfword of RS with its bytes reversed.
/// [PPC-Book1 p:46 s:3.3.5] `lmw` and `stmw` load or store the low-order words of the GPRs from RT or RS through 31.
/// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of the GPRs from RS up, with GPR 0 after GPR 31.
/// [PPC-Book2 p:25 s:3.3.2] `stwcx.` stores RS 32:63 into the word at EA when the reservation holds.
fn stores() -> Vec<(PpuInstruction, bool)> {
    use PpuInstruction as I;
    let (ra, rb) = (9, 10);
    vec![
        (I::Stw { rs: 3, ra, imm: 0 }, true),
        (I::Stwu { rs: 3, ra, imm: 0 }, true),
        (I::Stb { rs: 3, ra, imm: 0 }, true),
        (I::Sth { rs: 3, ra, imm: 0 }, true),
        (I::Stdu { rs: 3, ra, imm: 0 }, true),
        (I::Stwx { rs: 3, ra, rb }, true),
        (I::Stdx { rs: 3, ra, rb }, true),
        (I::Stdux { rs: 3, ra, rb }, true),
        (I::Stwbrx { rs: 3, ra, rb }, true),
        (I::Sthbrx { rs: 3, ra, rb }, true),
        (I::Stwcx { rs: 3, ra, rb }, true),
        (I::Stdcx { rs: 3, ra, rb }, true),
        (I::Stmw { rs: 2, ra, imm: 0 }, true),
        (I::Stmw { rs: 4, ra, imm: 0 }, false),
        (I::Stswi { rs: 2, ra, nb: 5 }, true),
        (I::Stswi { rs: 2, ra, nb: 4 }, false),
        (I::Stswx { rs: 4, ra, rb }, true),
    ]
}

/// The stores go through r9, and the `ld` reads through r1. The scan
/// keeps no address, so the `ld` can read each store.
#[test]
#[ignore = "the census carries no lost mark through storage yet, and the sound rule unresolves every held kernel"]
fn a_bctr_of_a_load_after_each_store_of_the_lost_gpr_leaves_every_site_unresolved() {
    let stores = stores();
    assert_eq!(stores.len(), 17);
    for (store, stores_r3) in stores {
        let words = stored_after_loop(word(store), &bctr_of_r4_after(ld(4, 1, SLOT)));
        let expected = if stores_r3 {
            EVERY_UNRESOLVED
        } else {
            WINDOW_RESOLVED
        };
        assert_eq!(sites(VECTOR, &words), expected, "{store:?}");
    }
}

/// Each word that writes r4, and whether it loads r4 from storage.
///
/// [PPC-Book1 p:33 s:3.3.2] A load writes RT, and its update form also writes the EA into RA.
/// [PPC-Book1 p:46 s:3.3.5] `lmw` and `stmw` load or store the low-order words of the GPRs from RT or RS through 31.
/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads NB bytes, or 32 when NB is 0, into GPRs from RT up, four bytes to each GPR, with GPR 0 after GPR 31.
/// [PPC-Book1 p:51 s:3.3.8] An arithmetic instruction places its result into RT.
fn loads() -> Vec<(PpuInstruction, bool)> {
    use PpuInstruction as I;
    let (ra, rb) = (9, 10);
    vec![
        (I::Ld { rt: 4, ra, imm: 0 }, true),
        (I::Ldu { rt: 4, ra, imm: 0 }, true),
        (I::Lwz { rt: 4, ra, imm: 0 }, true),
        (I::Lbz { rt: 4, ra, imm: 0 }, true),
        (I::Lhz { rt: 4, ra, imm: 0 }, true),
        (I::Lha { rt: 4, ra, imm: 0 }, true),
        (I::Lwa { rt: 4, ra, imm: 0 }, true),
        (I::Ldx { rt: 4, ra, rb }, true),
        (I::Lwzx { rt: 4, ra, rb }, true),
        (I::Lwbrx { rt: 4, ra, rb }, true),
        (I::Ldarx { rt: 4, ra, rb }, true),
        (I::Lwarx { rt: 4, ra, rb }, true),
        (I::Lmw { rt: 4, ra, imm: 0 }, true),
        (I::Lswi { rt: 4, ra, nb: 4 }, true),
        (
            I::Add {
                rt: 4,
                ra,
                rb,
                oe: false,
                rc: false,
            },
            false,
        ),
    ]
}

#[test]
#[ignore = "the census carries no lost mark through storage yet, and the sound rule unresolves every held kernel"]
fn a_bctr_of_each_load_after_a_store_of_the_lost_gpr_leaves_every_site_unresolved() {
    let loads = loads();
    assert_eq!(loads.len(), 15);
    for (load, loads_r4) in loads {
        let words = stored_after_loop(std(3, 1, SLOT), &bctr_of_r4_after(word(load)));
        let expected = if loads_r4 {
            EVERY_UNRESOLVED
        } else {
            WINDOW_RESOLVED
        };
        assert_eq!(sites(VECTOR, &words), expected, "{load:?}");
    }
}

/// The 17 paths bring one constant in r3 more than the join keeps, and
/// the `std` stores the lost r3. While the census carries no mark through
/// storage, the `ld` gives r4 no mark and no word takes the drop. A
/// census that carries the mark gives r4 the drop of the join. Then the
/// bound of the join rises, the join keeps every constant, and the `std`
/// stores no lost GPR.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_load_of_a_gpr_that_a_raised_join_keeps_resolves_once_the_bound_rises() {
    let mut words = writers(JOIN_LIMIT + 1, &[]);
    words.push(std(3, 1, SLOT));
    words.extend(bctr_of_r4_after(ld(4, 1, SLOT)));
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
    assert_eq!(losses(&words), HypercallLosses::default());
}

/// The `mtctr` at word 6 takes the r4 that the `ld` loads, and that value
/// names the loop join at word 2 past the raised limit.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
#[test]
#[ignore = "the census carries no lost mark through storage yet, and the sound rule unresolves every held kernel"]
fn a_bctr_of_a_load_of_a_stored_loop_gpr_names_the_loop_join_past_the_raised_limit() {
    let words = stored_after_loop(std(3, 1, SLOT), &bctr_of_r4_after(ld(4, 1, SLOT)));
    let drop = JoinDrop {
        join: at(2),
        register: JoinRegister::Gpr(3),
        brought: None,
    };
    let expected = HypercallLosses {
        lost_targets: vec![at(7)],
        dropped: vec![DroppedValue {
            address: at(6),
            drops: vec![drop],
            more: false,
        }],
    };
    assert_eq!(losses(&words), expected);
}
