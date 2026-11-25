use super::tests::word;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_NOP};

/// The kernel word at real address 0x40, below the interrupt vectors.
const LOW: u64 = 0x8000_0000_0000_0040;

/// A word that no value names except the constant that the code at
/// [`LOW`] builds.
const NAMED: u64 = 0x2_0000;

/// A byte of 0x40 between bytes that are not 0. No load of 2 or more
/// bytes of these bytes gives 0x40 or the real address of a word.
const BYTES: [u8; 3] = [0x11, 0x40, 0x22];

/// The address of the file bytes of each test.
const FILE: u64 = 0x3_0000;

/// A code address above 4 GiB. Only an 8-byte load gives it.
const HIGH: u64 = 0x8000_0001_0000_7000;

/// The code `lis r3,2; blr` at `vaddr`, which builds [`NAMED`] in r3,
/// and a `nop` at [`NAMED`].
fn builds_named_at(vaddr: u64) -> Code {
    let builds_named = word(PpuInstruction::Addis {
        rt: 3,
        ra: 0,
        imm: 2,
    });
    Code::new(vec![
        CodeSegment {
            vaddr,
            words: vec![builds_named, PPC_BLR],
        },
        CodeSegment {
            vaddr: NAMED,
            words: vec![PPC_NOP],
        },
    ])
}

/// An `lbz` of the middle byte gives 0x40, and a `bctr` in real mode with
/// it in CTR enters [`LOW`]. So the reach set holds [`LOW`], and its
/// `lis r3,2` names [`NAMED`]. The byte names no word itself.
///
/// [PPC-Book1 p:34 s:3.3.2] `lbz` puts the byte into RT bits 56:63 and sets RT bits 0:55 to 0.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn the_value_of_a_byte_load_starts_the_reach_set_and_names_no_word() {
    let code = builds_named_at(LOW);
    let files = [FileSegment {
        vaddr: FILE,
        bytes: &BYTES,
    }];
    let bound = indirect_entries(&[], &files, &code).expect("the code has no jump table");
    assert!(bound.lost_targets.is_empty());
    assert_eq!(bound.entries, BTreeSet::from([NAMED]));
}

/// For each case, only the named load form of the file bytes gives the
/// address of the code that builds [`NAMED`]. None of these gives it:
/// - a byte;
/// - a load of the other byte order;
/// - a load with the other extension;
/// - a word-aligned word or doubleword.
///
/// So the reach set holds that code, and its `lis r3,2` names [`NAMED`].
/// The load value names no word itself.
///
/// [PPC-Book1 p:38 s:3.3.2] `lwa` fills RT bits 0:31 with a copy of bit 0 of the loaded word.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn each_load_form_of_the_file_bytes_starts_the_reach_set_and_names_no_word() {
    let doubleword = HIGH.to_le_bytes();
    let cases: [(&[u8], u64, &str); 5] = [
        // 0x3440 is above the interrupt vectors.
        (
            &[0x40, 0x34],
            0x8000_0000_0000_3440,
            "byte-reversed halfword",
        ),
        (
            &[0x80, 0x40],
            0xFFFF_FFFF_FFFF_8040,
            "sign-extended halfword",
        ),
        (
            &[0x00, 0x00, 0x01, 0x23, 0x40],
            0x8000_0000_0001_2340,
            "word one byte past a word boundary",
        ),
        (
            &[0x40, 0x23, 0x01, 0x00],
            0x8000_0000_0001_2340,
            "byte-reversed word",
        ),
        (&doubleword, HIGH, "byte-reversed doubleword"),
    ];
    for (bytes, vaddr, load) in cases {
        let code = builds_named_at(vaddr);
        let files = [FileSegment { vaddr: FILE, bytes }];
        let bound = indirect_entries(&[], &files, &code).expect("the code has no jump table");
        assert!(bound.lost_targets.is_empty(), "{load}");
        assert_eq!(bound.entries, BTreeSet::from([NAMED]), "{load}");
    }
}
