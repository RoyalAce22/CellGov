use super::hypercall_ctr_copy_tests::call;
use super::hypercall_ctr_decrement_tests::{window, SC, VECTOR};
use super::hypercall_ctr_virtual_tests::{bctr, OUTSIDE};
use super::hypercall_entry_tests::{
    code, data_bytes, elf, ordinals, pointers, word, Segment, POINTERS,
};
use super::hypercall_real_mode_switch_tests::{lis_sf, rfid_stub, window as switch_window};
use super::hypercall_tests::{li, KERNEL};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The window's `nop`, between its `li r11` and `sc`.
const NOP_WORD: u64 = SC - 4;

/// The address of each `bcctr`, `bclr` or `rfid` whose target the scan of
/// `segments` loses.
fn lost_targets(segments: &[Segment]) -> Vec<u64> {
    hypercall_losses(&elf(0, segments))
        .expect("the ELF scans")
        .lost_targets
}

/// The kernel address of a routine that builds the real address of the
/// window's `nop`. Its real address is above 4 GiB, so no 2-byte or
/// 4-byte value of the file bytes is the address of one of its words.
const ROUTINE: u64 = KERNEL + 0x1_0000_7000;

/// The routine at [`ROUTINE`]: [`build_nop_real_address`], then `blr`.
fn routine() -> Segment {
    let mut words = build_nop_real_address().to_vec();
    words.push(PPC_BLR);
    code(ROUTINE, &words)
}

/// The word after the window's `blr`, where [`window`] puts its tail.
const TAIL: u64 = SC + 8;

/// `lis r3,0; ori r3,r3,0x3004`: the real address of the window's `nop`
/// in r3.
fn build_nop_real_address() -> [u32; 2] {
    [
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: 0x3004,
        }),
    ]
}

/// The 32-bit offset from `start` to the window's `nop`, as a relative
/// jump table entry at `start` holds it.
fn offset_to_nop(start: u64) -> u32 {
    u32::try_from(NOP_WORD.wrapping_sub(start) & 0xFFFF_FFFF).expect("a 32-bit offset")
}

/// The built constant 0x3004 sits after the window's `blr`, where no word
/// leads. So it names no word, although a real-mode `blr` at the vector
/// could enter its real alias.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_constant_that_only_an_unreached_word_builds_names_no_word() {
    let segments = [code(VECTOR, &[PPC_BLR]), window(&build_nop_real_address())];
    assert_eq!(ordinals(0, &segments), [(SC, Some(75))]);
}

/// A `b` at the vector leads to the same words, so their constant names
/// the real alias of 0x3004, the window's `nop`.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_constant_that_a_branch_from_the_vector_reaches_names_its_real_alias() {
    let to_tail = word(PpuInstruction::B {
        offset: i32::try_from(TAIL - VECTOR).expect("a near branch"),
        aa: false,
        link: false,
    });
    let mut tail = build_nop_real_address().to_vec();
    tail.push(PPC_BLR);
    let segments = [code(VECTOR, &[to_tail]), window(&tail)];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
    assert!(lost_targets(&segments).is_empty());
}

/// A code pointer names a call outside the vectors. The return point of
/// the call builds 0x3004, so the constant names the real alias of the
/// window's `nop`, which the `blr` at the vector can enter.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_constant_at_the_return_point_of_a_reached_call_names_its_real_alias() {
    let mut stub = vec![call()];
    stub.extend(build_nop_real_address());
    stub.push(PPC_BLR);
    let segments = [
        code(VECTOR, &[PPC_BLR]),
        code(OUTSIDE, &stub),
        pointers(&[OUTSIDE]),
        window(&[]),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
    assert!(lost_targets(&segments).is_empty());
}

/// The vector builds 0x5000 as 0xA00 shifted left by 3, so no word of the
/// image holds that value. Its real alias is the `bl` at [`OUTSIDE`], so
/// control reaches the `bctr` after it, which can reach any word.
///
/// [PPC-Book1 p:72 s:3.3.12.1] `rldicl` and `rldicr` rotate RS left by SH and AND it with MASK(MB, 63) or MASK(0, ME).
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_bctr_after_a_call_at_the_real_alias_of_a_built_constant_leaves_every_site_unresolved() {
    let shift_left_3 = word(PpuInstruction::Rldicr {
        ra: 3,
        rs: 3,
        sh: 3,
        me: 60,
        rc: false,
    });
    let segments = [
        code(VECTOR, &[li(3, 0xA00), shift_left_3, PPC_BLR]),
        code(OUTSIDE, &[call(), bctr()]),
        window(&[]),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
    assert_eq!(lost_targets(&segments), [OUTSIDE + 4]);
}

/// A `bl` and a `bctr` after the window's `blr` would bring a CTR that
/// the call can change. No word leads to them, so the `bctr` loses no
/// target.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_after_a_call_that_no_word_reaches_loses_no_target() {
    let segments = [window(&[call(), bctr()])];
    assert_eq!(ordinals(0, &segments), [(SC, Some(75))]);
}

/// A code pointer names the first word of the same `bl` and `bctr`, so
/// the `bctr` can reach any word.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_after_a_call_that_a_code_pointer_reaches_leaves_every_site_unresolved() {
    let segments = [
        code(OUTSIDE, &[call(), bctr()]),
        pointers(&[OUTSIDE]),
        window(&[]),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
    assert_eq!(lost_targets(&segments), [OUTSIDE + 4]);
}

/// The word after the `bctr` reads as a relative jump table entry to the
/// window's `nop`. No word leads to the `bctr`, so the table names no
/// word.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_jump_table_after_a_bctr_that_no_word_reaches_names_no_word() {
    let segments = [window(&[bctr(), offset_to_nop(TAIL + 4)])];
    assert_eq!(ordinals(0, &segments), [(SC, Some(75))]);
}

/// A code pointer names the `bctr`, so the table after it enters the
/// window's `nop`.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_jump_table_after_a_bctr_that_a_code_pointer_reaches_enters_its_target() {
    let segments = [
        code(OUTSIDE, &[bctr(), offset_to_nop(OUTSIDE + 4)]),
        pointers(&[OUTSIDE]),
        window(&[]),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
    assert!(lost_targets(&segments).is_empty());
}

/// The data holds the routine's address one byte past a word boundary.
/// The bound does not name such an address yet, but control can still
/// reach the routine through it. Only the 8-byte load at that byte gives
/// the address. So the routine's constant names the real alias of the
/// window's `nop`, which the `blr` at the vector can enter.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_routine_that_only_an_unaligned_pointer_names_still_names_its_constants() {
    let mut bytes = vec![0];
    bytes.extend(ROUTINE.to_be_bytes());
    let segments = [
        code(VECTOR, &[PPC_BLR]),
        data_bytes(POINTERS, bytes),
        routine(),
        window(&[]),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
    assert!(lost_targets(&segments).is_empty());
}

/// No word reaches the `rfid` stub until a code pointer names it, so
/// until then its switch puts no word into real mode, and the window's
/// tail does not reach the real address of the `nop`.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn an_rfid_that_no_word_reaches_puts_no_target_in_real_mode() {
    let stub = || rfid_stub(&[lis_sf(4)], &[]);
    assert_eq!(ordinals(0, &[stub(), switch_window(76)]), [(SC, Some(75))]);
    assert_eq!(
        ordinals(0, &[stub(), switch_window(76), pointers(&[OUTSIDE])]),
        [(SC, None)]
    );
}
