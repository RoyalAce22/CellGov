use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

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

/// `bdnz $+4`: a decrement of CTR on both paths to the next word.
fn bdnz_next() -> u32 {
    word(PpuInstruction::Bc {
        bo: 0b10000,
        bi: 0,
        offset: 4,
        aa: false,
        link: false,
    })
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// Gives `check` the CTR flow over `words` at [`BASE`] after its run,
/// with no constant, lost value or ranged value at any `mtctr`, and the
/// `bcctr` addresses that it finds lost.
///
/// The flow steps every word. A start enters each address of `starts`.
fn run(words: Vec<u32>, starts: &[u64], check: impl FnOnce(&CtrFlow<'_>, &BTreeSet<u64>)) {
    let code = Code::new(vec![CodeSegment { vaddr: BASE, words }]);
    let (constants, lost, ranged) = (BTreeMap::new(), BTreeSet::new(), BTreeSet::new());
    let mut reach = Reach::every(&code);
    reach.grow(starts.iter().copied());
    let writes = CtrWrites {
        constants: &constants,
        lost: &lost,
        ranged: &ranged,
    };
    let mut flow = CtrFlow::new(&code, writes, &reach);
    flow.run();
    let lost = flow.bcctr_targets(&mut BTreeMap::new());
    check(&flow, &lost);
}

/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
#[test]
fn a_decrement_keeps_a_state_with_no_path_and_loses_a_ctr_that_no_word_writes() {
    assert_eq!(Ctr::default().unproved(), Ctr::default());
    assert_eq!(Ctr::unwritten().unproved(), Ctr::lost());
}

/// The `bdnz` falls through with CTR at 0 in either flow. Its taken path
/// decrements the CTR at word 0: no path when no start enters word 0, and
/// a CTR that no word writes when a start enters it.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_bctr_after_a_bdnz_is_lost_only_when_a_start_enters_the_bdnz() {
    let zero = Ctr::written(at(0), Value::One(0));
    run(vec![bdnz_next(), bctr()], &[], |flow, lost| {
        assert_eq!(flow.at[0][0].ctr, Ctr::default());
        assert_eq!(flow.at[0][1].ctr, zero);
        assert!(lost.is_empty());
    });
    run(vec![bdnz_next(), bctr()], &[at(0)], |flow, lost| {
        assert_eq!(flow.at[0][0].ctr, Ctr::unwritten());
        let mut both = zero.clone();
        both.join(&Ctr::lost());
        assert_eq!(flow.at[0][1].ctr, both);
        assert_eq!(*lost, BTreeSet::from([at(1)]));
    });
}

/// An `mtctr` of r3, which holds no constant and no copy, copies a value
/// that no word of the scan writes, with or without a start.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr`, forms of `mtspr`, copy RS into LR or CTR.
#[test]
fn an_mtctr_of_a_gpr_with_no_constant_and_no_copy_makes_the_bctr_after_a_bdnz_lost() {
    let mtctr = word(PpuInstruction::Mtctr { rs: 3 });
    for starts in [Vec::new(), vec![at(0)]] {
        run(vec![mtctr, bdnz_next(), bctr()], &starts, |flow, lost| {
            assert_eq!(flow.at[0][1].ctr, Ctr::unwritten());
            assert_eq!(*lost, BTreeSet::from([at(2)]));
        });
    }
}

/// The flow steps the `bdnz` at word 0 with no path first. A start then
/// enters word 2, whose `b` goes back to word 0, so the CTR that no word
/// writes reaches the `bdnz` only after that step, and the decrement
/// makes the `bctr` lost.
///
/// [PPC-Book1 p:24 s:2.4] `b` reaches CIA + EXTS(LI || 0b00) when AA = 0.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_ctr_that_no_word_writes_joined_back_into_a_bdnz_makes_the_bctr_after_it_lost() {
    let back = word(PpuInstruction::B {
        offset: -8,
        aa: false,
        link: false,
    });
    run(vec![bdnz_next(), bctr(), back], &[at(2)], |flow, lost| {
        assert_eq!(flow.at[0][0].ctr, Ctr::unwritten());
        assert_eq!(*lost, BTreeSet::from([at(1)]));
    });
}

/// The `bl` at word 0 calls the routine at word 2. No start enters word
/// 2, and the flow does not follow CTR into a routine.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_call_target_starts_with_a_ctr_that_no_word_writes() {
    let call = word(PpuInstruction::B {
        offset: 8,
        aa: false,
        link: true,
    });
    run(
        vec![call, PPC_BLR, bdnz_next(), bctr()],
        &[],
        |flow, lost| {
            assert_eq!(flow.at[0][2].ctr, Ctr::unwritten());
            assert_eq!(*lost, BTreeSet::from([at(3)]));
        },
    );
}
