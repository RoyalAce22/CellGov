use cellgov_lv1::PAGE_TABLE_STORAGE_BASE;
use cellgov_ppu::mmu::Partition;
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_DATA_STORAGE;
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_ps3_abi::lv1::repository::lookup;

use super::super::from_entry::real_mode_partition;
use super::super::kernel_partition;
use super::*;

/// The 4.93 GameOS partition's real mode size, `bi.pu.1.rm_size`, from
/// the 4.93 repository capture.
const RM_SIZE_4_93: u64 = 0x0800_0000;

/// The floors' real mode size, `bi.pu.1.rm_size`, from the repository
/// dump [PS3DEVWIKI-REPO s:3.41_GameOS].
const RM_SIZE_FLOOR: u64 = 0x0100_0000;

const FLOORS: [&str; 3] = ["1.50", "1.94", "3.70"];

/// `std r3, ds(r4)`, with `ds` a multiple of 4.
const fn std_r3_r4(ds: i16) -> u32 {
    0xF864_0000 | (ds as u16 as u32)
}

/// `lis r4, imm`.
const fn lis_r4(imm: u16) -> u32 {
    0x3C80_0000 | imm as u32
}

/// The key `bi.pu.1.<name>` of the GameOS partition.
fn pu_1(name: &[u8]) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_GAME_OS,
        n1: first_field(b"bi", 0),
        n2: field(b"pu", 0),
        n3: 1,
        n4: field(name, 0),
    }
}

/// A kernel whose entry is the reset vector, so both threads store r3 at
/// `(hi << 16) + ds`, then spin. A data storage interrupt enters a spin
/// at its vector.
fn store_elf(hi: u16, ds: i16) -> Vec<u8> {
    let vector_word = ((VECTOR_DATA_STORAGE - 0x100) / 4) as usize;
    let mut words = vec![0u32; vector_word + 1];
    words[..3].copy_from_slice(&[lis_r4(hi), std_r3_r4(ds), B_SELF]);
    words[vector_word] = B_SELF;
    elf_image(BASE, 0, &words, 0x100)
}

/// Both threads' pcs after a run of `elf` at 4.93.
fn pcs_after(elf: &[u8]) -> (u64, u64) {
    let run = run_kernel_from_entry(elf, "4.93", 50).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    (thread0.pc, thread1.pc)
}

#[test]
fn the_4_93_kernel_partition_limits_real_mode_to_the_captured_rm_size() {
    let node = lookup(&pu_1(b"rm_size"), "4.93").expect("rm_size at 4.93");
    assert_eq!((node.v1, node.source.class()), (RM_SIZE_4_93, "console"));
    let partition = kernel_partition("4.93").expect("the 4.93 partition");
    assert_eq!(partition.rm_limit(), RM_SIZE_4_93);
    assert_eq!(partition.rm_offset(), 0);
    assert_eq!(partition.sdr1(), Partition::INITIAL.sdr1());
    assert_eq!(partition.lb(), Partition::INITIAL.lb());
}

#[test]
fn the_floor_kernel_partitions_keep_the_floor_rows_rm_size() {
    for fw in FLOORS {
        let partition = kernel_partition(fw).unwrap_or_else(|e| panic!("{fw}: {e}"));
        assert_eq!(partition.rm_limit(), RM_SIZE_FLOOR, "{fw}");
    }
}

/// A CellGov real address is a partition address, so the offset 0 holds
/// only while the region starts at partition address 0.
#[test]
fn each_served_real_mode_region_starts_at_partition_address_0() {
    for fw in FLOORS.into_iter().chain(["4.93"]) {
        let node = lookup(&pu_1(b"rm_addr"), fw).unwrap_or_else(|| panic!("rm_addr at {fw}"));
        assert_eq!(node.v1, 0, "{fw}");
    }
}

#[test]
fn a_firmware_with_no_real_mode_size_row_gets_no_partition_and_no_run() {
    let error = kernel_partition("0.00").unwrap_err();
    assert!(
        matches!(&error, KernelRunError::NoRealModeSize { firmware } if firmware == "0.00"),
        "{error}"
    );
    let error = run_kernel_from_entry(&kernel_elf(&[B_SELF]), "0.00", 4).unwrap_err();
    assert!(
        matches!(&error, KernelRunError::NoRealModeSize { firmware } if firmware == "0.00"),
        "{error}"
    );
}

/// [CBE-Handbook p:336 s:11.2] Table 11-1 has no 512 MB, 2 GB or 4 KB limit.
#[test]
fn a_real_mode_size_that_no_rmls_value_selects_is_refused() {
    for size in [512 << 20, 2 << 30, 1 << 12, RM_SIZE_4_93 + 0x1000] {
        let error = real_mode_partition("4.93", size).unwrap_err();
        assert!(
            matches!(&error, KernelRunError::RealModeSize { firmware, size: s }
                if firmware == "4.93" && *s == size),
            "{error}"
        );
    }
}

#[test]
fn a_real_mode_store_to_the_page_table_storage_takes_the_data_storage_interrupt_on_both_threads() {
    const { assert!(PAGE_TABLE_STORAGE_BASE >= RM_SIZE_4_93) };
    let elf = store_elf((PAGE_TABLE_STORAGE_BASE >> 16) as u16, 0);
    assert_eq!(pcs_after(&elf), (VECTOR_DATA_STORAGE, VECTOR_DATA_STORAGE));
}

#[test]
fn a_real_mode_store_at_the_rm_size_takes_the_data_storage_interrupt() {
    let elf = store_elf((RM_SIZE_4_93 >> 16) as u16, 0);
    assert_eq!(pcs_after(&elf), (VECTOR_DATA_STORAGE, VECTOR_DATA_STORAGE));
}

#[test]
fn a_real_mode_store_below_the_rm_size_and_inside_the_region_completes() {
    const { assert!(KERNEL_SPAN_LIMIT - 8 < RM_SIZE_4_93) };
    let elf = store_elf((KERNEL_SPAN_LIMIT >> 16) as u16, -8);
    let spin = 0x100 + 2 * 4;
    assert_eq!(pcs_after(&elf), (BASE + spin, spin));
}
