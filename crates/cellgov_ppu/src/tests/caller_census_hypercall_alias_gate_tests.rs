use super::hypercall_constant_join_tests::{
    addi, bctr, mtctr, EVERY_UNRESOLVED, NOWHERE, WINDOW_RESOLVED,
};
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, elf, pointers, word, Segment};
use super::hypercall_real_mode_switch_tests::{lis_sf, mtspr, RFID};
use super::hypercall_tests::KERNEL;
use super::*;
use crate::indirect_entry::IndirectBound;
use cellgov_ps3_abi::hw::ppu_supervisor::{SPR_SRR0, SPR_SRR1};

/// The kernel address of the code that a code pointer names. No real-mode
/// path goes there.
const STUB: u64 = KERNEL + 0x5000;

/// The real address of the target of the stub's `rfid`.
const TARGET: u16 = 0x6000;

/// The address of a data segment of 32-bit words.
const DATA: u64 = 0x2100_0000;

/// A data segment that holds each 32-bit word of `words`. Each word names
/// the word of the code at its real address.
fn named(words: &[u32]) -> Segment {
    data_bytes(
        DATA,
        words.iter().flat_map(|word| word.to_be_bytes()).collect(),
    )
}

/// `ld r3,0(r4)`.
fn load() -> u32 {
    word(PpuInstruction::Ld {
        rt: 3,
        ra: 4,
        imm: 0,
    })
}

/// `b .`.
fn spin() -> u32 {
    word(PpuInstruction::B {
        offset: 0,
        aa: false,
        link: false,
    })
}

/// The ELF of a stub at [`STUB`] that builds SRR0 [`TARGET`] in r3, gives
/// SRR1 an MSR value with SF at 1 and IR at 0, and runs `rfid`; of
/// `target` at the kernel address of [`TARGET`]; and of the two windows.
///
/// A data word names the real address of the stub's `ori`, and one names
/// the window's `nop`.
fn switch_elf(target: &[u32]) -> Vec<u8> {
    let stub = [
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: TARGET,
        }),
        mtspr(SPR_SRR0, 3),
        lis_sf(4),
        mtspr(SPR_SRR1, 4),
        RFID,
    ];
    let segments = [
        code(STUB, &stub),
        code(KERNEL + u64::from(TARGET), target),
        window(&[]),
        quiet(),
        pointers(&[STUB]),
        named(&[0x5004, 0x3004]),
    ];
    elf(0, &segments)
}

/// The address of the stub's `rfid`.
const STUB_RFID: u64 = STUB + 0x14;

/// The bound of `elf`, as the census scan builds it.
fn bound_of(elf: &[u8]) -> IndirectBound {
    let segments = pt_load_segments(elf).expect("the ELF has PT_LOAD segments");
    let code = Code::new(
        executable_segments(elf, &segments, validate_segment_extent).expect("valid segments"),
    );
    let files = file_segments(elf, &segments).expect("valid file bytes");
    indirect_entries(elf, &files, &code).expect("every jump table ends")
}

/// The ordinal of each site of `elf`, by address.
fn sites_of(elf: &[u8]) -> Vec<(u64, Option<u16>)> {
    scan_hypercalls(elf)
        .expect("the ELF scans")
        .into_iter()
        .map(|site| (site.address, site.ordinal))
        .collect()
}

/// No `bcctr` or `bclr` runs in real mode, so no branch reaches the real
/// address of the `ori`, and the switch scan keeps the SRR0 constant.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_alias_that_no_real_mode_branch_reaches_keeps_the_switch_bounded() {
    let elf = switch_elf(&[spin()]);
    let bound = bound_of(&elf);
    assert_eq!(bound.unbounded_switches, BTreeSet::new());
    assert_eq!(
        bound.real_mode_switches,
        BTreeSet::from([u64::from(TARGET)])
    );
    assert_eq!(sites_of(&elf), WINDOW_RESOLVED);
}

/// The switch puts a `bctr` of a loaded address in real mode. That
/// `bctr` can enter the real address of the `ori`, so the bound runs
/// again with that word as a join, and the switch scan then holds no
/// SRR0 constant.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches the next instruction from SRR0.
#[test]
fn a_real_mode_branch_with_an_unknown_target_makes_the_switch_unbounded() {
    let elf = switch_elf(&[load(), mtctr(3), bctr()]);
    let bound = bound_of(&elf);
    assert_eq!(bound.unbounded_switches, BTreeSet::from([STUB_RFID]));
}

/// The kernel address of the loop of [`loop_elf`].
const LOOP: u64 = KERNEL + 0x7000;

/// The ELF of a loop at [`LOOP`] that steps r3 by 0x10 from 0x5000 and
/// branches through it, with a code pointer to it, a data word that names
/// the real address of its `mtctr`, the two windows and `vector` at
/// [`VECTOR`].
fn loop_elf(vector: &[u32]) -> Vec<u8> {
    let words = [
        addi(3, 0, NOWHERE.cast_signed()),
        addi(3, 3, 0x10),
        bc(0b01100, -4),
        mtctr(3),
        bctr(),
    ];
    let segments = [
        code(VECTOR, vector),
        code(LOOP, &words),
        window(&[]),
        quiet(),
        pointers(&[LOOP]),
        named(&[0x700C]),
    ];
    elf(0, &segments)
}

/// The `mtctr` is an entry only for a branch in real mode, and none runs
/// there, so r3 keeps the class of the loop.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_loop_mtctr_that_only_a_real_alias_names_keeps_the_class_with_no_real_mode_branch() {
    assert_eq!(sites_of(&loop_elf(&[spin()])), WINDOW_RESOLVED);
}

/// The `bctr` of a loaded address at the vector runs in real mode, so it
/// can enter the `mtctr` with any value in r3.
///
/// [PPC-Book3 p:65 s:5.5] Figure 35: each interrupt sets MSR\[IR\] to 0.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_loop_mtctr_that_only_a_real_alias_names_is_an_entry_with_a_real_mode_branch() {
    let vector = [load(), mtctr(3), bctr()];
    assert_eq!(sites_of(&loop_elf(&vector)), EVERY_UNRESOLVED);
}
