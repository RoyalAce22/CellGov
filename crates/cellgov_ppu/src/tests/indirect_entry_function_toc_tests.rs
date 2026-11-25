use std::collections::BTreeSet;

use super::tests::{at, word};
use super::toc_tests::{build_from_toc, descriptor_bytes, DATA, TOC};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;

/// A second TOC, which builds the address one word past the one that
/// [`TOC`] builds.
pub(super) const OTHER_TOC: u64 = TOC + 4;

/// The bound of an image with the code `words` and one descriptor for
/// each `(first word index, TOC)` of `functions`.
///
/// The code is also file bytes, so the bound reads jump tables from it.
pub(super) fn bound_of(words: &[u32], functions: &[(u64, u64)]) -> BTreeSet<u64> {
    indirect_bound_of(words, functions).entries
}

/// The [`IndirectBound`] of the image that [`bound_of`] reads.
pub(super) fn indirect_bound_of(words: &[u32], functions: &[(u64, u64)]) -> IndirectBound {
    let code = Code::new(vec![CodeSegment {
        vaddr: at(0),
        words: words.to_vec(),
    }]);
    let code_bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    let data: Vec<u8> = functions
        .iter()
        .flat_map(|&(first, toc)| descriptor_bytes(at(first), toc, 0))
        .collect();
    let files = [
        FileSegment {
            vaddr: at(0),
            bytes: &code_bytes,
        },
        FileSegment {
            vaddr: DATA,
            bytes: &data,
        },
    ];
    indirect_entries(&[], &files, &code).expect("every jump table ends")
}

/// `b` from word `from` to word `to`.
pub(super) fn b(from: u64, to: u64) -> u32 {
    word(PpuInstruction::B {
        offset: i32::try_from(to.wrapping_sub(from) as i64 * 4).expect("the offset fits"),
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

/// The words `addis r9,r2,..; addi r9,r9,..; mtctr r9; bctr`, which
/// branch to word `target` when r2 holds `toc`.
pub(super) fn branch_through(target: u64, toc: u64) -> Vec<u32> {
    let mut words = build_from_toc(at(target), toc).to_vec();
    words.push(word(PpuInstruction::Mtctr { rs: 9 }));
    words.push(bctr());
    words
}

/// `words` with `nop` after it up to `len` words.
pub(super) fn padded(mut words: Vec<u32>, len: usize) -> Vec<u32> {
    words.resize(len, PPC_NOP);
    words
}

#[test]
fn an_address_built_from_the_toc_after_an_unconditional_branch_in_the_function_is_named() {
    let mut words = vec![b(0, 2), PPC_NOP];
    words.extend(branch_through(8, TOC));
    assert_eq!(
        bound_of(&padded(words, 10), &[(0, TOC)]),
        BTreeSet::from([at(0), at(8)])
    );
}

#[test]
fn an_address_built_from_the_toc_in_a_case_that_only_a_jump_table_reaches_is_named() {
    // The table after the first `bctr` holds one offset, to word 4.
    let mut words = vec![word(PpuInstruction::Mtctr { rs: 3 }), bctr(), 8, 0];
    words.extend(branch_through(10, TOC));
    assert_eq!(
        bound_of(&padded(words, 12), &[(0, TOC)]),
        BTreeSet::from([at(0), at(4), at(10)])
    );
}

#[test]
fn an_address_built_from_the_toc_after_a_word_that_writes_r2_is_named() {
    let mut words = vec![
        word(PpuInstruction::B {
            offset: 36,
            aa: false,
            link: true,
        }),
        word(PpuInstruction::Ld {
            rt: 2,
            ra: 1,
            imm: 40,
        }),
    ];
    words.extend(branch_through(8, TOC));
    let mut words = padded(words, 9);
    words.push(word(PpuInstruction::Bclr {
        bo: 0b10100,
        bi: 0,
        link: false,
    }));
    assert_eq!(
        bound_of(&words, &[(0, TOC)]),
        BTreeSet::from([at(0), at(8)])
    );
}

#[test]
fn a_word_that_no_function_reaches_gets_no_toc() {
    let mut words = vec![word(PpuInstruction::Bclr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })];
    words.extend(branch_through(8, TOC));
    assert_eq!(
        bound_of(&padded(words, 10), &[(0, TOC)]),
        BTreeSet::from([at(0)])
    );
}

#[test]
fn a_word_that_functions_with_two_tocs_reach_names_the_address_from_each_toc() {
    let mut words = vec![b(0, 3), b(1, 3), PPC_NOP];
    words.extend(branch_through(9, TOC));
    assert_eq!(
        bound_of(&padded(words, 12), &[(0, TOC), (1, OTHER_TOC)]),
        BTreeSet::from([at(0), at(1), at(9), at(10)])
    );
}

/// Word 0 is a function with [`OTHER_TOC`] that branches to itself and
/// has no indirect branch. Word 1 is a function with [`TOC`] whose taken
/// `bc` and fall-through path both reach word 3, so word 3 runs only
/// with [`TOC`] in r2.
///
/// [OFW-4.93 lv2_kernel a:0x80000000002765e4-0x8000000000276600] A call through a descriptor loads LR from its first doubleword and r2 from its second.
#[test]
fn a_word_that_only_a_function_with_one_toc_reaches_names_the_address_from_that_toc_only() {
    let mut words = vec![
        b(0, 0),
        word(PpuInstruction::Bc {
            bo: 0b01100,
            bi: 0,
            offset: 8,
            aa: false,
            link: false,
        }),
        PPC_NOP,
    ];
    words.extend(branch_through(10, TOC));
    assert_eq!(
        bound_of(&padded(words, 12), &[(0, OTHER_TOC), (1, TOC)]),
        BTreeSet::from([at(0), at(1), at(10)])
    );
}

#[test]
fn a_branch_to_the_first_word_of_another_function_gives_it_the_toc_of_the_branch_too() {
    let mut words = vec![b(0, 2), PPC_NOP, b(2, 4), PPC_NOP];
    words.extend(branch_through(10, TOC));
    assert_eq!(
        bound_of(&padded(words, 12), &[(0, OTHER_TOC), (2, TOC)]),
        BTreeSet::from([at(0), at(2), at(10), at(11)])
    );
}

#[test]
fn an_address_built_from_the_toc_in_a_routine_that_only_a_call_reaches_is_named() {
    // Word 0 calls word 4, which no descriptor names.
    let mut words = vec![
        word(PpuInstruction::B {
            offset: 16,
            aa: false,
            link: true,
        }),
        word(PpuInstruction::Bclr {
            bo: 0b10100,
            bi: 0,
            link: false,
        }),
        PPC_NOP,
        PPC_NOP,
    ];
    words.extend(branch_through(10, TOC));
    assert_eq!(
        bound_of(&padded(words, 12), &[(0, TOC)]),
        BTreeSet::from([at(0), at(10)])
    );
}
