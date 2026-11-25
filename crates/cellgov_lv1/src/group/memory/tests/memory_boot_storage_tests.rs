use super::storage::ALLOCATED_STORAGE_BYTES;
use super::tests::{hypervisor_4_93, kernel_sizes_4_93, request};
use super::*;

/// The firmware versions that the repository nodes of the GameOS
/// partition name.
const REPOSITORY_FIRMWARE: &[&str] = &["1.50", "1.94", "3.15", "3.41", "3.70", "4.93"];

#[test]
fn the_4_93_boot_allocations_each_get_storage_from_the_storage_base() {
    let (first, second) = kernel_sizes_4_93();
    let mut hv = hypervisor_4_93();
    hv.dispatch(&request([first, 20, 0, 0]));
    hv.dispatch(&request([second, 16, 0, 0]));
    let regions = hv.memory().regions();
    assert_eq!(regions.len(), 2);
    assert_eq!(
        hv.memory().storage_of(&regions[0]),
        Some(ALLOCATED_STORAGE_BASE)
    );
    assert_eq!(
        hv.memory().storage_of(&regions[1]),
        Some(ALLOCATED_STORAGE_BASE + first)
    );
}

#[test]
fn the_storage_holds_the_partition_memory_at_each_firmware_that_gives_it() {
    assert_eq!(partition_pool("4.93"), Some(ALLOCATED_STORAGE_BYTES));
    for firmware in REPOSITORY_FIRMWARE {
        if let Some(pool) = partition_pool(firmware) {
            assert!(pool <= ALLOCATED_STORAGE_BYTES, "{firmware}: {pool:#x}");
        }
    }
}
