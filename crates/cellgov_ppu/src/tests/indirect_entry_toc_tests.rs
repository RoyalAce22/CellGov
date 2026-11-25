use std::collections::BTreeSet;

use super::tests::{at, word};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;

/// The address of the data segment that holds the descriptors.
pub(super) const DATA: u64 = 0x2_0000;

/// The TOC of the descriptors. No file byte lies at it.
pub(super) const TOC: u64 = DATA + 0x8000;

/// The `addis` and `addi` immediates that add `target - toc` to the TOC.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) and `addis` adds EXTS(SI || 0x0000).
fn toc_halves(target: u64, toc: u64) -> (i16, i16) {
    let offset = target.wrapping_sub(toc) as i64;
    let low = offset as i16;
    let high = i16::try_from((offset - i64::from(low)) >> 16).expect("the offset fits addis");
    (high, low)
}

/// The words `addis r9,r2,..; addi r9,r9,..`, which build `target` in r9
/// from the TOC `toc`.
pub(super) fn build_from_toc(target: u64, toc: u64) -> [u32; 2] {
    let (high, low) = toc_halves(target, toc);
    [
        word(PpuInstruction::Addis {
            rt: 9,
            ra: 2,
            imm: high,
        }),
        word(PpuInstruction::Addi {
            rt: 9,
            ra: 9,
            imm: low,
        }),
    ]
}

/// The bytes of one function descriptor.
pub(super) fn descriptor_bytes(entry: u64, toc: u64, environment: u64) -> Vec<u8> {
    [entry, toc, environment]
        .iter()
        .flat_map(|doubleword| doubleword.to_be_bytes())
        .collect()
}

/// The bound of an image with the code `words` and the data `data`, which
/// holds the only file bytes.
fn bound_with_data(words: &[u32], data: &[u8]) -> BTreeSet<u64> {
    bound_with_data_at(words, DATA, data)
}

/// The bound of an image with the code `words` and the data `data` at
/// `vaddr`, which holds the only file bytes.
fn bound_with_data_at(words: &[u32], vaddr: u64, data: &[u8]) -> BTreeSet<u64> {
    let code = Code::new(vec![CodeSegment {
        vaddr: at(0),
        words: words.to_vec(),
    }]);
    let files = [FileSegment { vaddr, bytes: data }];
    indirect_entries(&[], &files, &code)
        .expect("every jump table ends")
        .entries
}

/// The code `addis r9,r2,..; addi r9,r9,..; mtctr r9; bctr`, which
/// branches to word 8 when r2 holds `TOC`.
fn branch_through_toc() -> Vec<u32> {
    let mut words = build_from_toc(at(8), TOC).to_vec();
    words.push(word(PpuInstruction::Mtctr { rs: 9 }));
    words.push(word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    }));
    words.resize(10, PPC_NOP);
    words
}

#[test]
fn a_code_address_built_from_the_toc_that_a_descriptor_gives_is_named() {
    let data = descriptor_bytes(at(0), TOC, 0);
    assert_eq!(
        bound_with_data(&branch_through_toc(), &data),
        BTreeSet::from([at(0), at(8)])
    );
}

#[test]
fn a_code_address_built_from_the_toc_of_a_descriptor_in_an_unaligned_segment_is_named() {
    // The segment starts one byte past a word, so its first word-aligned
    // address is three bytes in.
    let mut data = vec![0u8; 3];
    data.extend(descriptor_bytes(at(0), TOC, 0));
    assert_eq!(
        bound_with_data_at(&branch_through_toc(), DATA + 1, &data),
        BTreeSet::from([at(0), at(8)])
    );
}

#[test]
fn a_jump_table_based_at_an_address_built_from_the_toc_adds_its_targets() {
    let table = DATA + descriptor::SIZE as u64;
    let mut words = build_from_toc(table, TOC).to_vec();
    words.extend([
        word(PpuInstruction::Lwzx {
            rt: 0,
            ra: 9,
            rb: 10,
        }),
        word(PpuInstruction::Add {
            rt: 0,
            ra: 0,
            rb: 9,
            oe: false,
            rc: false,
        }),
        word(PpuInstruction::Mtctr { rs: 0 }),
        word(PpuInstruction::Bcctr {
            bo: 0b10100,
            bi: 0,
            link: false,
        }),
        PPC_NOP,
        PPC_NOP,
    ]);
    let mut data = descriptor_bytes(at(0), TOC, 0);
    for target in [at(6), at(7)] {
        let offset = i32::try_from(target.wrapping_sub(table) as i64).expect("the offset fits");
        data.extend(offset.to_be_bytes());
    }
    data.extend(0u32.to_be_bytes());
    assert_eq!(
        bound_with_data(&words, &data),
        BTreeSet::from([at(0), at(6), at(7)])
    );
}

#[test]
fn a_code_address_is_named_from_each_toc_of_two_descriptors_that_disagree() {
    let mut data = descriptor_bytes(at(0), TOC, 0);
    data.extend(descriptor_bytes(at(0), TOC + 4, 0));
    assert_eq!(
        bound_with_data(&branch_through_toc(), &data),
        BTreeSet::from([at(0), at(8), at(9)])
    );
}

#[test]
fn no_code_address_is_named_from_the_toc_of_a_descriptor_with_a_nonzero_environment() {
    let data = descriptor_bytes(at(0), TOC, 1);
    assert_eq!(
        bound_with_data(&branch_through_toc(), &data),
        BTreeSet::from([at(0)])
    );
}
