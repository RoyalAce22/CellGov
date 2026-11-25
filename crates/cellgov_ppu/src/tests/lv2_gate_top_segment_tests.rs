//! A capability check whose error path reaches its return through an
//! absolute branch, in a function that lies in the top 32 MiB.

use super::*;

/// The lowest address that a negative I-form displacement reaches.
const BASE: u64 = 0xFFFF_FFFF_FE00_0000;

/// A permission record with `mask` behind the TOC slot at offset 0.
fn record(mask: u32) -> (Vec<u8>, Vec<LoadSegment>, u64) {
    let mut elf = vec![0u8; 0x400];
    elf[0x100..0x108].copy_from_slice(&0x2200u64.to_be_bytes());
    elf[0x200..0x204].copy_from_slice(&mask.to_be_bytes());
    let segments = vec![LoadSegment {
        index: 0,
        file_offset: 0,
        vaddr: 0x2000,
        filesz: 0x400,
        memsz: 0x400,
        executable: true,
        writable: true,
        readable: true,
    }];
    (elf, segments, 0x2100)
}

/// `ld r4; bl; clrlwi r3,r3,24; lis r9,0x8001; cmpwi cr7,r3,0;
/// ori r9,r9,3; beq cr7,+4`, then `ba` over a `bctr` to `extsw r3,r9; blr`.
fn check_through_absolute_branch() -> Vec<Option<PpuInstruction>> {
    // [PPC-Book1 p:24 s:2.4] With AA = 1 the target is EXTS(LI || 0b00), so
    // the displacement of word 9 at BASE is the signed value of its address.
    let to_word_nine = (BASE + 9 * 4) as i64;
    vec![
        Some(PpuInstruction::Ld {
            rt: 4,
            ra: 2,
            imm: 0,
        }),
        Some(PpuInstruction::B {
            offset: 4,
            aa: false,
            link: true,
        }),
        Some(PpuInstruction::Rlwinm {
            ra: 3,
            rs: 3,
            sh: 0,
            mb: 24,
            me: 31,
            rc: false,
        }),
        Some(PpuInstruction::Addis {
            rt: 9,
            ra: 0,
            imm: 0x8001u16 as i16,
        }),
        Some(PpuInstruction::Cmpwi {
            bf: 7,
            ra: 3,
            imm: 0,
        }),
        Some(PpuInstruction::Ori {
            ra: 9,
            rs: 9,
            imm: 3,
        }),
        Some(PpuInstruction::Bc {
            bo: 12,
            bi: 30,
            offset: 4,
            aa: false,
            link: false,
        }),
        Some(PpuInstruction::B {
            offset: i32::try_from(to_word_nine).expect("I-form displacement"),
            aa: true,
            link: false,
        }),
        Some(PpuInstruction::Bcctr {
            bo: 20,
            bi: 0,
            link: false,
        }),
        Some(PpuInstruction::Extsw {
            ra: 3,
            rs: 9,
            rc: false,
        }),
        Some(PpuInstruction::Bclr {
            bo: 20,
            bi: 0,
            link: false,
        }),
    ]
}

#[test]
fn error_path_through_a_negative_absolute_branch_yields_a_gated_candidate() {
    let (elf, segments, toc) = record(0x4000_0000);
    assert_eq!(
        find_candidates(&elf, &segments, toc, BASE, &check_through_absolute_branch()),
        [Candidate::Gated {
            mask: 0x4000_0000,
            fail_errno: errno::CELL_ENOSYS.code,
        }]
    );
}
