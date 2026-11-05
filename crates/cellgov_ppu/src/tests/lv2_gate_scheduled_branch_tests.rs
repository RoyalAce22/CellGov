//! A capability check whose branch the compiler scheduled past argument
//! setup, as firmware 4.93 lays out ordinal 355's implementation.

use super::*;

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

fn addi(rt: u8, ra: u8, imm: i16) -> Option<PpuInstruction> {
    Some(PpuInstruction::Addi { rt, ra, imm })
}

/// `ld r4; bl; clrlwi r0,r3,24; addi r7; cmpwi cr7,r0,0`, then `between`,
/// then `bne cr7; extsw r3,r0; blr`.
fn check_with(between: &[Option<PpuInstruction>]) -> Vec<Option<PpuInstruction>> {
    let mut code = vec![
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
            ra: 0,
            rs: 3,
            sh: 0,
            mb: 24,
            me: 31,
            rc: false,
        }),
        addi(7, 1, 144),
        Some(PpuInstruction::Cmpwi {
            bf: 7,
            ra: 0,
            imm: 0,
        }),
    ];
    code.extend_from_slice(between);
    code.extend([
        Some(PpuInstruction::Bc {
            bo: 4,
            bi: 30,
            offset: 0x20,
            aa: false,
            link: false,
        }),
        Some(PpuInstruction::Extsw {
            ra: 3,
            rs: 0,
            rc: false,
        }),
        Some(PpuInstruction::Bclr {
            bo: 20,
            bi: 0,
            link: false,
        }),
    ]);
    code
}

/// The 4.93 setup: `lis r0,0x8001; li r4; li r5; li r6; mr r3,r31;
/// ori r0,r0,3`.
fn scheduled_setup() -> Vec<Option<PpuInstruction>> {
    vec![
        Some(PpuInstruction::Addis {
            rt: 0,
            ra: 0,
            imm: 0x8001u16 as i16,
        }),
        addi(4, 0, 8192),
        addi(5, 0, 5),
        addi(6, 0, 256),
        Some(PpuInstruction::Or {
            ra: 3,
            rs: 31,
            rb: 31,
            rc: false,
        }),
        Some(PpuInstruction::Ori {
            ra: 0,
            rs: 0,
            imm: 3,
        }),
    ]
}

#[test]
fn a_branch_six_setup_instructions_past_the_compare_is_a_gate() {
    let (elf, segments, toc) = record(0x4000_0000);
    assert_eq!(
        find_candidates(
            &elf,
            &segments,
            toc,
            0x2000,
            &check_with(&scheduled_setup())
        ),
        [Candidate::Gated {
            mask: 0x4000_0000,
            fail_errno: errno::CELL_ENOSYS.code,
        }]
    );
}

#[test]
fn a_load_among_the_setup_past_four_instructions_hides_the_branch() {
    let (elf, segments, toc) = record(0x4000_0000);
    let mut between = scheduled_setup();
    between[1] = Some(PpuInstruction::Lwz {
        rt: 4,
        ra: 1,
        imm: 0,
    });
    assert!(find_candidates(&elf, &segments, toc, 0x2000, &check_with(&between)).is_empty());
}

#[test]
fn a_branch_eight_instructions_past_the_compare_behind_only_setup_is_a_gate() {
    let (elf, segments, toc) = record(0x4000_0000);
    let mut between = scheduled_setup();
    between.push(addi(8, 0, 1));
    assert_eq!(between.len(), 7);
    assert_eq!(
        find_candidates(&elf, &segments, toc, 0x2000, &check_with(&between)),
        [Candidate::Gated {
            mask: 0x4000_0000,
            fail_errno: errno::CELL_ENOSYS.code,
        }]
    );
}

#[test]
fn an_errno_built_just_before_a_branch_eight_past_the_compare_is_found() {
    let (elf, segments, toc) = record(0x4000_0000);
    let between = vec![
        addi(4, 0, 8192),
        addi(5, 0, 5),
        addi(6, 0, 256),
        addi(8, 0, 1),
        Some(PpuInstruction::Or {
            ra: 3,
            rs: 31,
            rb: 31,
            rc: false,
        }),
        Some(PpuInstruction::Addis {
            rt: 0,
            ra: 0,
            imm: 0x8001u16 as i16,
        }),
        Some(PpuInstruction::Ori {
            ra: 0,
            rs: 0,
            imm: 3,
        }),
    ];
    assert_eq!(between.len(), 7);
    assert_eq!(
        find_candidates(&elf, &segments, toc, 0x2000, &check_with(&between)),
        [Candidate::Gated {
            mask: 0x4000_0000,
            fail_errno: errno::CELL_ENOSYS.code,
        }]
    );
}

#[test]
fn a_branch_five_instructions_past_the_compare_behind_a_load_is_not_a_gate() {
    let (elf, segments, toc) = record(0x4000_0000);
    let between = vec![
        Some(PpuInstruction::Addis {
            rt: 0,
            ra: 0,
            imm: 0x8001u16 as i16,
        }),
        Some(PpuInstruction::Lwz {
            rt: 4,
            ra: 1,
            imm: 0,
        }),
        addi(5, 0, 5),
        Some(PpuInstruction::Ori {
            ra: 0,
            rs: 0,
            imm: 3,
        }),
    ];
    assert!(find_candidates(&elf, &segments, toc, 0x2000, &check_with(&between)).is_empty());
}

#[test]
fn a_branch_more_than_eight_instructions_past_the_compare_is_not_a_gate() {
    let (elf, segments, toc) = record(0x4000_0000);
    let mut between = scheduled_setup();
    between.extend([addi(8, 0, 1), addi(9, 0, 2)]);
    assert_eq!(between.len(), 8);
    assert!(find_candidates(&elf, &segments, toc, 0x2000, &check_with(&between)).is_empty());
}
