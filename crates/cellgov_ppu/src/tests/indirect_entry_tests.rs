use super::*;
use crate::instruction::encode::encode;
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;

/// The address of word 0 of the code in each test.
const BASE: u64 = 0x1_0000;

pub(super) fn word(instruction: PpuInstruction) -> u32 {
    encode(&instruction).expect("the instruction encodes")
}

/// The address of word `index` of the code.
pub(super) fn at(index: u64) -> u64 {
    BASE + 4 * index
}

/// The bound of an image whose only segment is the code `words`.
///
/// With `readable` set, the code is also the file bytes that the bound
/// reads for doublewords and jump tables.
pub(super) fn bound(words: &[u32], readable: bool) -> BTreeSet<u64> {
    let code = Code::new(vec![CodeSegment {
        vaddr: BASE,
        words: words.to_vec(),
    }]);
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    let files = [FileSegment {
        vaddr: BASE,
        bytes: &bytes,
    }];
    let files: &[FileSegment<'_>] = if readable { &files } else { &[] };
    indirect_entries(&[], files, &code)
        .expect("every jump table ends")
        .entries
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// The jump table after `bctr` starts at word 1, so an entry that
/// reaches word `index` holds the offset `4 * (index - 1)`.
fn entry_to(index: u32) -> u32 {
    4 * (index - 1)
}

#[test]
fn a_jump_table_ends_at_the_first_offset_that_reaches_no_code() {
    let words = [
        bctr(),
        entry_to(5),
        entry_to(6),
        0x0010_0000,
        entry_to(7),
        PPC_NOP,
        PPC_NOP,
        PPC_NOP,
    ];
    assert_eq!(bound(&words, true), BTreeSet::from([at(5), at(6)]));
}

#[test]
fn a_jump_table_that_runs_past_the_entry_limit_is_refused() {
    let table = |entries: u64| {
        let mut words = vec![bctr()];
        let back_to_bctr = 0xFFFF_FFFC;
        words.extend((0..entries).map(|_| back_to_bctr));
        words.push(PPC_NOP);
        let code = Code::new(vec![CodeSegment {
            vaddr: BASE,
            words: words.clone(),
        }]);
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        let files = [FileSegment {
            vaddr: BASE,
            bytes: &bytes,
        }];
        indirect_entries(&[], &files, &code).map(|bound| bound.entries)
    };
    assert_eq!(
        table(JUMP_TABLE_LIMIT).expect("a table of the limit ends"),
        BTreeSet::from([at(0)])
    );
    assert!(matches!(
        table(JUMP_TABLE_LIMIT + 1),
        Err(CallerScanError::JumpTableTooLong { start }) if start == at(1)
    ));
}

#[test]
fn a_jump_table_ends_at_an_offset_that_reaches_back_into_it() {
    // The second entry, at word 2, reaches the table's start (offset 0)
    // or its own word (offset 4).
    let table = |back: u32| {
        let words = [
            bctr(),
            entry_to(5),
            back,
            entry_to(6),
            PPC_NOP,
            PPC_NOP,
            PPC_NOP,
        ];
        bound(&words, true)
    };
    assert_eq!(table(0), BTreeSet::from([at(5)]));
    assert_eq!(table(4), BTreeSet::from([at(5)]));
}

#[test]
fn a_constant_is_forgotten_when_another_word_writes_its_register() {
    let building = |between: u32| {
        let words = [
            word(PpuInstruction::Addis {
                rt: 5,
                ra: 0,
                imm: 1,
            }),
            between,
            word(PpuInstruction::Ori {
                ra: 5,
                rs: 5,
                imm: 0x10,
            }),
            PPC_NOP,
            PPC_NOP,
        ];
        bound(&words, false)
    };
    let load_r5 = word(PpuInstruction::Lwz {
        rt: 5,
        ra: 3,
        imm: 0,
    });
    assert_eq!(building(PPC_NOP), BTreeSet::from([at(0), at(4)]));
    assert_eq!(building(load_r5), BTreeSet::from([at(0)]));
}

#[test]
fn mflr_after_a_branch_and_link_to_the_next_word_names_that_word() {
    let words = [
        word(PpuInstruction::Bc {
            bo: 0b10100,
            bi: 31,
            offset: 4,
            aa: false,
            link: true,
        }),
        word(PpuInstruction::Mflr { rt: 9 }),
        word(PpuInstruction::Addi {
            rt: 9,
            ra: 9,
            imm: 12,
        }),
        PPC_NOP,
        PPC_NOP,
    ];
    assert_eq!(bound(&words, false), BTreeSet::from([at(1), at(4)]));
}
