use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_NOP};

use super::*;
use crate::instruction::encode::encode;

const BASE: u64 = 0x8000_0000_0010_0000;

fn word(instruction: PpuInstruction) -> u32 {
    encode(&instruction).expect("the instruction encodes")
}

/// The address of word `index` of the code.
fn at(index: u64) -> u64 {
    BASE + 4 * index
}

fn mtctr_r3() -> u32 {
    word(PpuInstruction::Mtctr { rs: 3 })
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// `b` or `bl` to the word `offset` bytes on.
fn branch(offset: i32, link: bool) -> u32 {
    word(PpuInstruction::B {
        offset,
        aa: false,
        link,
    })
}

/// Gives `check` the CTR flow over `words` at [`BASE`] after its run, the
/// `bcctr` targets that it finds and the `bcctr` addresses that it finds
/// lost. The `mtctr` at word `ranged` copies a value of a class whose
/// words the bound names.
fn run(
    words: Vec<u32>,
    ranged: u64,
    check: impl FnOnce(&CtrFlow<'_>, &BTreeMap<u64, BTreeSet<u64>>, &BTreeSet<u64>),
) {
    let code = Code::new(vec![CodeSegment { vaddr: BASE, words }]);
    let (constants, lost) = (BTreeMap::new(), BTreeSet::new());
    let ranged = BTreeSet::from([(0, at(ranged))]);
    let reach = Reach::every(&code);
    let writes = CtrWrites {
        constants: &constants,
        lost: &lost,
        ranged: &ranged,
    };
    let mut flow = CtrFlow::new(&code, writes, &reach);
    flow.run();
    let mut targets = BTreeMap::new();
    let lost = flow.bcctr_targets(&mut targets);
    check(&flow, &targets, &lost);
}

/// Word 3 branches back to word 0 with a CTR of the class from the
/// `mtctr` at word 2, after the flow first stepped word 0 with no CTR. A
/// `bdz` at word 0 decrements the class value, which can leave the class,
/// so the `bctr` at word 1 is lost. A `nop` at word 0 keeps the class
/// value, which names no target and is not lost.
///
/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a01t decrements CTR, then branches if the decremented CTR M:63 is 0.
#[test]
fn a_ctr_of_a_class_that_a_back_branch_brings_is_lost_after_a_bdz_and_kept_after_a_nop() {
    let bdz = word(PpuInstruction::Bc {
        bo: 0b10010,
        bi: 0,
        offset: 4,
        aa: false,
        link: false,
    });
    let back = branch(-12, false);
    run(
        vec![bdz, bctr(), mtctr_r3(), back],
        2,
        |flow, targets, lost| {
            assert_eq!(flow.at[0][1].ctr, Ctr::lost());
            assert!(targets.is_empty());
            assert_eq!(*lost, BTreeSet::from([at(1)]));
        },
    );
    run(
        vec![PPC_NOP, bctr(), mtctr_r3(), back],
        2,
        |flow, targets, lost| {
            assert_eq!(flow.at[0][1].ctr, Ctr::ranged());
            assert!(targets.is_empty());
            assert!(lost.is_empty());
        },
    );
}

/// A call can return with any CTR, so the `bctr` after a `bl` to a
/// routine is lost. A `bl` to the next word runs no routine and keeps the
/// class value.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_ctr_of_a_class_is_lost_after_a_call_and_kept_after_a_bl_to_the_next_word() {
    let words = |offset| vec![mtctr_r3(), branch(offset, true), bctr(), PPC_BLR];
    run(words(8), 0, |flow, targets, lost| {
        assert_eq!(flow.at[0][2].ctr, Ctr::lost());
        assert!(targets.is_empty());
        assert_eq!(*lost, BTreeSet::from([at(2)]));
    });
    run(words(4), 0, |flow, targets, lost| {
        assert_eq!(flow.at[0][2].ctr, Ctr::ranged());
        assert!(targets.is_empty());
        assert!(lost.is_empty());
    });
}
