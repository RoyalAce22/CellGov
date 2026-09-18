use super::*;

fn synthetic_record(mask: u32) -> (Vec<u8>, Vec<LoadSegment>, u64) {
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

fn gate_sequence() -> Vec<Option<PpuInstruction>> {
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
            offset: 8,
            aa: false,
            link: false,
        }),
        Some(PpuInstruction::Nop),
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
fn nonzero_permission_record_yields_a_gated_candidate() {
    let (elf, segments, toc) = synthetic_record(0x4000_0000);
    assert_eq!(
        find_candidates(&elf, &segments, toc, 0x2000, &gate_sequence()),
        [Candidate::Gated {
            mask: 0x4000_0000,
            fail_errno: errno::CELL_ENOSYS.code,
        }]
    );
}

#[test]
fn zero_permission_record_yields_an_ungated_candidate() {
    let (elf, segments, toc) = synthetic_record(0);
    assert_eq!(
        find_candidates(&elf, &segments, toc, 0x2000, &gate_sequence()),
        [Candidate::Ungated]
    );
}

#[test]
fn permission_record_with_an_unrecognized_tail_is_not_classified() {
    let (mut elf, segments, toc) = synthetic_record(0x4000_0000);
    elf[0x207] = 1;
    assert!(find_candidates(&elf, &segments, toc, 0x2000, &gate_sequence()).is_empty());
}

#[test]
fn entry_thunk_targets_its_branch_destination() {
    assert_eq!(
        entry_thunk_target(
            0x2000,
            &PpuInstruction::B {
                offset: -0x10,
                aa: false,
                link: false,
            },
        ),
        Some(0x1ff0)
    );
}
