use super::hypercall_ctr_decrement_tests::{quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_tests::KERNEL;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The kernel address of a routine that builds a constant in r3.
const ROUTINE: u64 = KERNEL + 0x7000;

/// A word that runs only in virtual mode: no real-mode word reaches it.
const OUTSIDE: u64 = KERNEL + 0x5000;

/// The ordinals when a path enters the second window's `sc` with r11
/// unset.
const ONLY_QUIET_UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, Some(75)), (QUIET_SC, None)];

/// `lis rt,0; ori rt,rt,value`.
fn build(rt: u8, value: u16) -> [u32; 2] {
    [
        word(PpuInstruction::Addis { rt, ra: 0, imm: 0 }),
        word(PpuInstruction::Ori {
            ra: rt,
            rs: rt,
            imm: value,
        }),
    ]
}

/// `std r3,0(r1); ld rt,0(r1)`: a copy of r3 through memory, which the
/// scan does not follow.
fn through_memory(rt: u8) -> [u32; 2] {
    [
        word(PpuInstruction::Std {
            rs: 3,
            ra: 1,
            imm: 0,
        }),
        word(PpuInstruction::Ld { rt, ra: 1, imm: 0 }),
    ]
}

fn mtctr(rs: u8) -> u32 {
    word(PpuInstruction::Mtctr { rs })
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// [`build`] of 0x4004 into r3, [`through_memory`] into `rt`, then
/// `tail`, at `at`.
fn loaded_back(at: u64, rt: u8, tail: &[u32]) -> Segment {
    let mut words = build(3, 0x4004).to_vec();
    words.extend_from_slice(&through_memory(rt));
    words.extend_from_slice(tail);
    code(at, &words)
}

/// The routine returns 0x4004 in r3, where the stub's own r3 held
/// 0x3000. The `bctr` then enters real address 0x4004, the second
/// window's `sc`, with r11 unset.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_bctr_of_a_constant_a_routine_builds_enters_the_word_at_its_real_address() {
    let offset = i32::try_from(ROUTINE.wrapping_sub(VECTOR).cast_signed()).expect("in reach");
    let mut stub = build(3, 0x3000).to_vec();
    stub.extend([
        word(PpuInstruction::B {
            offset,
            aa: false,
            link: true,
        }),
        mtctr(3),
        bctr(),
    ]);
    let mut routine = build(3, 0x4004).to_vec();
    routine.push(PPC_BLR);
    let segments = [
        code(VECTOR, &stub),
        window(&[]),
        quiet(),
        code(ROUTINE, &routine),
    ];
    assert_eq!(ordinals(0, &segments), ONLY_QUIET_UNRESOLVED);
}

/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_bctr_of_a_constant_loaded_back_from_memory_enters_the_word_at_its_real_address() {
    let segments = [
        loaded_back(VECTOR, 4, &[mtctr(4), bctr()]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), ONLY_QUIET_UNRESOLVED);
}

/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_blr_of_a_constant_loaded_back_from_memory_enters_the_word_at_its_real_address() {
    let mtlr = word(PpuInstruction::Mtlr { rs: 0 });
    let segments = [
        loaded_back(VECTOR, 0, &[mtlr, PPC_BLR]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), ONLY_QUIET_UNRESOLVED);
}

/// No `bcctr` or `bclr` runs in real mode, and a branch in virtual mode
/// reaches only the word at the address itself.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_virtual_mode_bctr_of_a_constant_leaves_the_word_at_its_real_address_resolved() {
    let segments = [
        loaded_back(OUTSIDE, 4, &[mtctr(4), bctr()]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(
        ordinals(0, &segments),
        [(SC, Some(75)), (QUIET_SC, Some(77))]
    );
}

/// The first word of a function that a descriptor names: a `bctr`.
const FUNCTION: u64 = KERNEL + 0x1_0000;

/// The data segment that holds the function's descriptor.
const DESCRIPTORS: u64 = KERNEL + 0x2_0000;

/// The TOC that the descriptor gives. No file byte lies at it.
const TOC: u64 = DESCRIPTORS + 0x8000;

/// A word outside each function, which a real-mode `bctr` reaches only
/// at the real address of a constant.
const LANDING: u64 = KERNEL + 0x6000;

/// `addis r9,r2,..; addi r9,r9,..`, which build `target` in r9 when r2
/// holds [`TOC`].
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) and `addis` adds EXTS(SI || 0x0000).
fn from_toc(target: u64) -> [u32; 2] {
    let offset = target.wrapping_sub(TOC).cast_signed();
    let low = offset as i16;
    let high = i16::try_from((offset - i64::from(low)) >> 16).expect("the offset fits addis");
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

/// The function's `bctr` gives the word at the real address of 0x6000
/// its TOC. From that TOC the word builds the address of the window's
/// `nop`, so a path enters the window's `sc` with r11 unset.
///
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR, so r2 at the target is r2 at the branch.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_word_at_the_real_address_of_a_constant_takes_the_toc_of_an_indirect_branch() {
    let mut stub = build(3, 0x6000).to_vec();
    stub.extend([mtctr(3), bctr()]);
    let mut landing = from_toc(SC - 4).to_vec();
    landing.push(PPC_BLR);
    let descriptor: Vec<u8> = [FUNCTION, TOC, 0]
        .iter()
        .flat_map(|doubleword| doubleword.to_be_bytes())
        .collect();
    let segments = [
        code(VECTOR, &stub),
        window(&[]),
        code(LANDING, &landing),
        code(FUNCTION, &[bctr()]),
        data_bytes(DESCRIPTORS, descriptor),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
}
