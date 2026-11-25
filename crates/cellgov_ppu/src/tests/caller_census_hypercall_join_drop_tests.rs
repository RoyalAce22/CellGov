use super::hypercall_constant_join_tests::{
    bctr, lr_writers, mflr, mtctr, sites, twenty_pass_loop, writers, EVERY_UNRESOLVED, JOIN_LIMIT,
    WINDOW_RESOLVED,
};
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, VECTOR};
use super::hypercall_entry_tests::{code, elf};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The most constants that a raised bound keeps in one register at one
/// join word.
pub(super) const RAISED_LIMIT: u16 = 64;

/// What [`hypercall_losses`] gives for the window, the second window and
/// `words` at `at`.
fn losses(at: u64, words: &[u32]) -> HypercallLosses {
    let segments = [code(at, words), window(&[]), quiet()];
    hypercall_losses(&elf(0, &segments)).expect("the ELF scans")
}

/// The address of the word at `index` of the code at [`VECTOR`].
fn at(index: usize) -> u64 {
    VECTOR + 4 * index as u64
}

/// The losses of a code whose `mtctr`, `bclr` or `rfid` at `taker`
/// takes a value that the joins `drops` dropped, and whose indirect
/// branch at `target` can reach a dropped constant.
fn lost(target: u64, taker: u64, drops: &[JoinDrop]) -> HypercallLosses {
    HypercallLosses {
        lost_targets: vec![target],
        dropped: vec![DroppedValue {
            address: taker,
            drops: drops.to_vec(),
            more: false,
        }],
    }
}

/// A drop past the raised limit of `register` at the join word at
/// `index`.
fn past(index: usize, register: JoinRegister) -> JoinDrop {
    JoinDrop {
        join: at(index),
        register,
        brought: None,
    }
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_more_paths_bring_than_the_join_limit_resolves_once_the_bound_rises() {
    let mut words = writers(JOIN_LIMIT + 1, &[]);
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
    assert_eq!(losses(VECTOR, &words), HypercallLosses::default());
}

/// The 64 paths bring as many constants in r3 as a raised bound keeps,
/// so the bound of the join rises to 64 and the join drops none.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_as_many_paths_bring_as_the_raised_limit_resolves_once_the_bound_rises() {
    let mut words = writers(RAISED_LIMIT, &[]);
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
    assert_eq!(losses(VECTOR, &words), HypercallLosses::default());
}

/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_that_more_lr_writers_reach_than_the_join_limit_resolves_once_the_bound_rises() {
    let mut words = lr_writers(JOIN_LIMIT + 1);
    words.push(PPC_BLR);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The `mflr` copies the origin of the lost LR into r5, so the lost
/// `mtctr` names the join of LR, and its bound rises.
///
/// [PPC-Book1 p:82 s:3.3.13] `mflr` copies LR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_mflr_copy_of_an_lr_past_the_join_limit_resolves_once_the_bound_rises() {
    let mut words = lr_writers(JOIN_LIMIT + 1);
    words.extend([mflr(5), mtctr(5), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// Each of the 65 writers takes three words, so the `mtctr` at word 195
/// is the join word.
#[test]
fn a_bctr_of_a_gpr_that_more_paths_bring_than_the_raised_limit_names_its_join() {
    let writers_words = 3 * usize::from(RAISED_LIMIT + 1);
    let mut words = writers(RAISED_LIMIT + 1, &[]);
    words.extend([mtctr(3), bctr()]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
    let mtctr_at = at(writers_words);
    let drop = past(writers_words, JoinRegister::Gpr(3));
    assert_eq!(
        losses(VECTOR, &words),
        lost(at(writers_words + 1), mtctr_at, &[drop])
    );
}

/// Each of the 65 LR writers takes four words, so the `blr` at word 260
/// is the join word.
#[test]
fn a_blr_that_more_lr_writers_reach_than_the_raised_limit_names_the_join_of_lr() {
    let writers_words = 4 * usize::from(RAISED_LIMIT + 1);
    let mut words = lr_writers(RAISED_LIMIT + 1);
    words.push(PPC_BLR);
    let blr_at = at(writers_words);
    let drop = past(writers_words, JoinRegister::Lr);
    assert_eq!(losses(VECTOR, &words), lost(blr_at, blr_at, &[drop]));
}

/// The `mflr` at the join word copies the origin of LR into r5, so the
/// `mtctr` names the join of LR.
///
/// [PPC-Book1 p:82 s:3.3.13] `mflr` copies LR into RT.
#[test]
fn a_bctr_of_an_mflr_copy_of_an_lr_past_the_raised_limit_names_the_join_of_lr() {
    let writers_words = 4 * usize::from(RAISED_LIMIT + 1);
    let mut words = lr_writers(RAISED_LIMIT + 1);
    words.extend([mflr(5), mtctr(5), bctr()]);
    let drop = past(writers_words, JoinRegister::Lr);
    assert_eq!(
        losses(VECTOR, &words),
        lost(at(writers_words + 2), at(writers_words + 1), &[drop])
    );
}

/// The loop brings one constant more than each bound to its `addi` at
/// word 2, so each raise doubles the bound until it passes the raised
/// limit.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_steps_names_the_loop_join_past_the_raised_limit() {
    let mut words = twenty_pass_loop();
    words.extend([mtctr(3), bctr()]);
    let drop = past(2, JoinRegister::Gpr(3));
    assert_eq!(losses(VECTOR, &words), lost(at(5), at(4), &[drop]));
}

/// Two loops step r3, at the `addi` words 2 and 7. The `blt` at word 4
/// skips the second loop to the `mtctr` at word 9, so the `mtctr` joins
/// the marks of both loops.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
#[test]
fn a_join_of_two_lost_paths_names_the_joins_of_both() {
    let mut words = twenty_pass_loop();
    words.push(bc(0b01100, 4 * 5));
    words.extend(twenty_pass_loop());
    words.extend([mtctr(3), bctr()]);
    let drops = [past(2, JoinRegister::Gpr(3)), past(7, JoinRegister::Gpr(3))];
    assert_eq!(losses(VECTOR, &words), lost(at(10), at(9), &drops));
}

/// Seventeen loops step r3, and each loop but the last skips to the
/// `mtctr` after the last loop. A mark names at most 16 drops, so the
/// value that the `mtctr` takes names 16 of the loop drops and has more.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
#[test]
fn a_value_that_more_joins_drop_than_a_mark_names_has_more() {
    const LOOPS: i16 = 17;
    let mut words = Vec::new();
    // Each later loop takes five words with its skip, and the last
    // loop takes four.
    for later in (1..LOOPS).rev() {
        words.extend(twenty_pass_loop());
        words.push(bc(0b01100, 20 * later));
    }
    words.extend(twenty_pass_loop());
    let mtctr_word = words.len();
    words.extend([mtctr(3), bctr()]);
    let found = losses(VECTOR, &words);
    assert_eq!(found.lost_targets, vec![at(mtctr_word + 1)]);
    assert_eq!(found.dropped.len(), 1);
    let value = &found.dropped[0];
    assert_eq!(value.address, at(mtctr_word));
    assert_eq!(value.drops.len(), 16);
    assert!(value
        .drops
        .iter()
        .all(|drop| drop.register == JoinRegister::Gpr(3)));
    assert!(value.more);
}
