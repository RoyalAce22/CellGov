//! Guest-heap placement: the three floors `alloc_base` must clear.

use super::place_guest_heap;
use crate::prx::PrxLoadInfo;

/// ELF64 big-endian header plus one PT_LOAD at `(vaddr, memsz)`.
/// Only the headers matter -- `elf_user_region_end` never reads
/// segment contents.
fn elf64_be_with_load(vaddr: u64, memsz: u64) -> Vec<u8> {
    const EHDR: usize = 64;
    const PHENT: usize = 56;
    const PT_LOAD: u32 = 1;
    let mut out = vec![0u8; EHDR + PHENT];
    out[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    out[4] = 2; // ELFCLASS64
    out[5] = 2; // ELFDATA2MSB
    out[6] = 1; // EV_CURRENT
    out[32..40].copy_from_slice(&(EHDR as u64).to_be_bytes()); // e_phoff
    out[54..56].copy_from_slice(&(PHENT as u16).to_be_bytes()); // e_phentsize
    out[56..58].copy_from_slice(&1u16.to_be_bytes()); // e_phnum
    out[EHDR..EHDR + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    out[EHDR + 16..EHDR + 24].copy_from_slice(&vaddr.to_be_bytes());
    out[EHDR + 32..EHDR + 40].copy_from_slice(&memsz.to_be_bytes()); // p_filesz
    out[EHDR + 40..EHDR + 48].copy_from_slice(&memsz.to_be_bytes()); // p_memsz
    out
}

/// [`place_guest_heap`] with the refusal unwrapped: every case here
/// places inside the 32-bit space.
fn placement_of(
    elf: &[u8],
    prx_modules: &[PrxLoadInfo],
    code_floor: u32,
) -> super::MemoryPlacement {
    place_guest_heap(elf, prx_modules, code_floor, &crate::NullSink)
        .expect("the fixtures place inside u32")
}

fn prx_ending_at(data_end: u64) -> PrxLoadInfo {
    PrxLoadInfo {
        name: "test.sprx".to_string(),
        stem: "test".to_string(),
        base: 0,
        data_end,
        toc: 0,
        relocs_applied: 0,
        module_start: None,
        module_stop: None,
    }
}

#[test]
fn the_title_image_wins_when_it_reaches_highest() {
    let elf = elf64_be_with_load(0x1_0000, 0x1_0000);
    let p = placement_of(&elf, &[prx_ending_at(0x1_5000)], 0x1_2000);
    assert_eq!(p.user_region_end, 0x2_0000);
    assert_eq!(
        p.alloc_floor, 0x2_0000,
        "the image end is the highest floor"
    );
    assert_eq!(p.alloc_base, 0x2_0000);
}

#[test]
fn the_code_floor_wins_when_it_reaches_highest() {
    // An HLE trampoline span above the image would otherwise be
    // handed out by sys_memory_allocate.
    let elf = elf64_be_with_load(0x1_0000, 0x1_0000);
    let p = placement_of(&elf, &[prx_ending_at(0x1_5000)], 0x10_0000);
    assert_eq!(p.alloc_floor, 0x10_0000, "the code floor is the highest");
    assert_eq!(p.alloc_base, 0x10_0000);
}

#[test]
fn the_firmware_set_wins_when_it_reaches_highest() {
    // A PRX loaded above both the image and the trampolines: an
    // allocation below its data_end would land inside a loaded module.
    let elf = elf64_be_with_load(0x1_0000, 0x1_0000);
    let p = placement_of(&elf, &[prx_ending_at(0x80_0000)], 0x1_2000);
    assert_eq!(p.prx_region_end, 0x80_0000);
    assert_eq!(p.alloc_floor, 0x80_0000, "the firmware set is the highest");
    assert_eq!(p.alloc_base, 0x80_0000);
}

#[test]
fn the_highest_of_several_modules_sets_the_firmware_floor() {
    let elf = elf64_be_with_load(0x1_0000, 0x1_0000);
    let modules = [
        prx_ending_at(0x30_0000),
        prx_ending_at(0x90_0000),
        prx_ending_at(0x50_0000),
    ];
    let p = placement_of(&elf, &modules, 0x1_2000);
    assert_eq!(p.prx_region_end, 0x90_0000);
    assert_eq!(p.alloc_floor, 0x90_0000);
}

#[test]
fn a_floor_inside_a_page_rounds_the_base_up_to_the_next() {
    // 64K granularity: one byte past a boundary costs a whole page.
    let elf = elf64_be_with_load(0x1_0000, 0x1_0001);
    let p = placement_of(&elf, &[], 0);
    assert_eq!(p.alloc_floor, 0x2_0001);
    assert_eq!(p.alloc_base, 0x3_0000);
}

#[test]
fn a_boot_with_nothing_resident_still_starts_above_the_null_page() {
    // No parseable image, no firmware, no trampolines: the base must
    // not be 0, or the first allocation would hand out the null page.
    let p = placement_of(&[], &[], 0);
    assert_eq!(p.user_region_end, 0);
    assert_eq!(p.prx_region_end, 0);
    assert_eq!(p.alloc_floor, 0);
    assert_eq!(p.alloc_base, 0x1_0000);
}
