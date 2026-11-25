use cellgov_lv1::real_mode_size;
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_DATA_STORAGE;

use super::*;

/// The firmwares whose real mode size, `bi.pu.1.rm_size`, comes from the
/// repository dump [PS3DEVWIKI-REPO s:3.41_GameOS].
const FLOORS: [&str; 3] = ["1.50", "1.94", "3.70"];

/// `lis r4, imm`.
const fn lis_r4(imm: u16) -> u32 {
    0x3C80_0000 | imm as u32
}

/// `std r3, ds(r4)`, with `ds` a multiple of 4.
const fn std_r3_r4(ds: i16) -> u32 {
    0xF864_0000 | (ds as u16 as u32)
}

/// `ld r5, ds(r4)`, with `ds` a multiple of 4.
const fn ld_r5_r4(ds: i16) -> u32 {
    0xE8A4_0000 | (ds as u16 as u32)
}

/// The offset of the spin in the text of [`access_elf`]: the word after
/// the accesses.
const SPIN: u64 = 0x100 + 3 * 4;

/// A kernel whose entry is the reset vector, so both threads set r4 to
/// `hi << 16`, make the accesses `first` and `second`, then spin. A data
/// storage interrupt enters a spin at its vector.
fn access_elf(hi: u16, first: u32, second: u32) -> Vec<u8> {
    let vector_word = ((VECTOR_DATA_STORAGE - 0x100) / 4) as usize;
    let mut words = vec![0u32; vector_word + 1];
    words[..4].copy_from_slice(&[lis_r4(hi), first, second, B_SELF]);
    words[vector_word] = B_SELF;
    elf_image(BASE, 0, &words, 0x100)
}

/// Both threads' pcs after a run of `elf` at `fw` that reaches the step
/// cap.
fn pcs_after(elf: &[u8], fw: &str) -> (u64, u64) {
    let run = run_kernel_from_entry(elf, fw, 50).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap, "{fw}");
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    (thread0.pc, thread1.pc)
}

/// The real mode size at `fw`, and the high half of its address.
fn rm_size_and_hi(fw: &str) -> (u64, u16) {
    let size = real_mode_size(fw).unwrap_or_else(|| panic!("the real mode size at {fw}"));
    let hi = u16::try_from(size >> 16).expect("lis holds the size's high half");
    assert!(hi < 0x8000, "{fw}: lis sign-extends 0x{hi:04x}");
    assert_eq!(
        u64::from(hi) << 16,
        size,
        "{fw}: lis forms the size exactly"
    );
    (size, hi)
}

#[test]
fn a_4_93_real_mode_store_and_load_past_the_kernel_span_limit_and_just_below_the_rm_size_read_memory(
) {
    let (size, hi) = rm_size_and_hi("4.93");
    assert!(size - 8 >= KERNEL_SPAN_LIMIT, "0x{size:x}");
    let elf = access_elf(hi, std_r3_r4(-8), ld_r5_r4(-8));
    assert_eq!(pcs_after(&elf, "4.93"), (BASE + SPIN, SPIN));
}

#[test]
fn a_4_93_real_mode_load_at_the_rm_size_takes_the_data_storage_interrupt() {
    let (_, hi) = rm_size_and_hi("4.93");
    let elf = access_elf(hi, ld_r5_r4(0), B_SELF);
    assert_eq!(
        pcs_after(&elf, "4.93"),
        (VECTOR_DATA_STORAGE, VECTOR_DATA_STORAGE)
    );
}

#[test]
fn at_each_floor_a_real_mode_store_and_load_just_below_the_rm_size_read_memory() {
    for fw in FLOORS {
        let (_, hi) = rm_size_and_hi(fw);
        let elf = access_elf(hi, std_r3_r4(-8), ld_r5_r4(-8));
        assert_eq!(pcs_after(&elf, fw), (BASE + SPIN, SPIN), "{fw}");
    }
}

#[test]
fn at_each_floor_the_partition_limit_gives_a_real_mode_access_at_the_rm_size_the_data_storage_interrupt(
) {
    for fw in FLOORS {
        let (size, hi) = rm_size_and_hi(fw);
        assert!(size < KERNEL_SPAN_LIMIT, "{fw}: 0x{size:x}");
        for access in [std_r3_r4(0), ld_r5_r4(0)] {
            let elf = access_elf(hi, access, B_SELF);
            assert_eq!(
                pcs_after(&elf, fw),
                (VECTOR_DATA_STORAGE, VECTOR_DATA_STORAGE),
                "{fw}: 0x{access:08x}"
            );
        }
    }
}
