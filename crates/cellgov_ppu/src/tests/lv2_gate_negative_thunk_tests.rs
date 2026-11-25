//! An entry thunk that is an absolute branch with a negative displacement,
//! into an implementation in the top 32 MiB.

use super::*;
use crate::instruction::encode::encode;

/// The lowest address that a negative I-form displacement reaches.
const BASE: u64 = 0xFFFF_FFFF_FE00_0000;
const THUNK: u64 = 0x2000;
const TOC: u64 = 0x2100;
/// The file offset of the segment at [`BASE`].
const TOP_FILE_OFFSET: usize = 0x400;

/// `ld r4,toc_offset(r2); bl; clrlwi r3,r3,24; lis r9,0x8001;
/// cmpwi cr7,r3,0; ori r9,r9,3; beq cr7,+8; nop; extsw r3,r9; blr`.
fn check(toc_offset: i16) -> Vec<PpuInstruction> {
    vec![
        PpuInstruction::Ld {
            rt: 4,
            ra: 2,
            imm: toc_offset,
        },
        PpuInstruction::B {
            offset: 4,
            aa: false,
            link: true,
        },
        PpuInstruction::Rlwinm {
            ra: 3,
            rs: 3,
            sh: 0,
            mb: 24,
            me: 31,
            rc: false,
        },
        PpuInstruction::Addis {
            rt: 9,
            ra: 0,
            imm: 0x8001u16 as i16,
        },
        PpuInstruction::Cmpwi {
            bf: 7,
            ra: 3,
            imm: 0,
        },
        PpuInstruction::Ori {
            ra: 9,
            rs: 9,
            imm: 3,
        },
        PpuInstruction::Bc {
            bo: 12,
            bi: 30,
            offset: 8,
            aa: false,
            link: false,
        },
        PpuInstruction::Ori {
            ra: 0,
            rs: 0,
            imm: 0,
        },
        PpuInstruction::Extsw {
            ra: 3,
            rs: 9,
            rc: false,
        },
        PpuInstruction::Bclr {
            bo: 20,
            bi: 0,
            link: false,
        },
    ]
}

fn put_words(elf: &mut [u8], file_offset: usize, instructions: &[PpuInstruction]) {
    for (index, instruction) in instructions.iter().enumerate() {
        let word = encode(instruction).expect("encodable instruction");
        let at = file_offset + index * 4;
        elf[at..at + 4].copy_from_slice(&word.to_be_bytes());
    }
}

/// A `ba BASE` thunk at [`THUNK`] with an ungated check after it, and a
/// check gated on `0x4000_0000` at [`BASE`].
fn image() -> (Vec<u8>, Vec<LoadSegment>) {
    let top = check(0);
    let mut elf = vec![0u8; TOP_FILE_OFFSET + top.len() * 4];
    // [PPC-Book1 p:24 s:2.4] With AA = 1 the target is EXTS(LI || 0b00), so
    // the displacement that reaches BASE is the signed value of BASE.
    let thunk = PpuInstruction::B {
        offset: i32::try_from(BASE as i64).expect("I-form displacement"),
        aa: true,
        link: false,
    };
    put_words(&mut elf, 0, &[thunk]);
    put_words(&mut elf, 4, &check(8));
    elf[0x100..0x108].copy_from_slice(&0x2200u64.to_be_bytes());
    elf[0x108..0x110].copy_from_slice(&0x2220u64.to_be_bytes());
    elf[0x200..0x204].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    put_words(&mut elf, TOP_FILE_OFFSET, &top);
    let segments = vec![
        LoadSegment {
            index: 0,
            file_offset: 0,
            vaddr: THUNK,
            filesz: 0x400,
            memsz: 0x400,
            executable: true,
            writable: true,
            readable: true,
        },
        LoadSegment {
            index: 1,
            file_offset: TOP_FILE_OFFSET as u64,
            vaddr: BASE,
            filesz: (top.len() * 4) as u64,
            memsz: (top.len() * 4) as u64,
            executable: true,
            writable: false,
            readable: true,
        },
    ];
    (elf, segments)
}

#[test]
fn negative_absolute_entry_thunk_targets_the_sign_extended_displacement() {
    assert_eq!(
        entry_thunk_target(
            THUNK,
            &PpuInstruction::B {
                offset: -0x0200_0000,
                aa: true,
                link: false,
            },
        ),
        Some(BASE)
    );
}

#[test]
fn gate_row_comes_from_the_target_of_a_negative_absolute_thunk() {
    let (elf, segments) = image();
    assert_eq!(
        scan_target(&elf, &segments, TOC, THUNK),
        Lv2Gate::Gated {
            reads: Lv2GateRead::ControlFlags1(0x4000_0000),
            fail_errno: errno::CELL_ENOSYS.code,
        }
    );
}
