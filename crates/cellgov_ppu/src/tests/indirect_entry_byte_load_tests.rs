use super::tests::word;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_NOP};

/// The kernel word at real address 0x40, below the interrupt vectors.
const LOW: u64 = 0x8000_0000_0000_0040;

/// A word that only the constant that the code at [`LOW`] builds names.
const NAMED: u64 = 0x2_0000;

/// A byte of 0x40 between bytes that are not 0. No load of 2 or more
/// bytes of these bytes gives 0x40 or the real address of a word.
const BYTES: [u8; 3] = [0x11, 0x40, 0x22];

/// An `lbz` of the middle byte gives 0x40, and a `bctr` in real mode with
/// it in CTR enters [`LOW`]. So the reach set holds [`LOW`], and its
/// `lis r3,2` names [`NAMED`].
///
/// [PPC-Book1 p:34 s:3.3.2] `lbz` puts the byte into RT bits 56:63 and sets RT bits 0:55 to 0.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn the_value_of_a_byte_load_starts_the_reach_set() {
    let builds_named = word(PpuInstruction::Addis {
        rt: 3,
        ra: 0,
        imm: 2,
    });
    let code = Code::new(vec![
        CodeSegment {
            vaddr: LOW,
            words: vec![builds_named, PPC_BLR],
        },
        CodeSegment {
            vaddr: NAMED,
            words: vec![PPC_NOP],
        },
    ]);
    let files = [FileSegment {
        vaddr: 0x3_0000,
        bytes: &BYTES,
    }];
    let bound = indirect_entries(&[], &files, &code).expect("the code has no jump table");
    assert!(bound.lost_targets.is_empty());
    assert!(bound.entries.contains(&NAMED));
}
