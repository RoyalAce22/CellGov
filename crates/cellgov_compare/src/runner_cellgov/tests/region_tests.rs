//! Named-region extraction across address spaces with zero-fill on unresolvable requests.

use super::*;
use cellgov_mem::{PageSize, Region, RegionAccess};

fn desc(name: &str, space: AddressSpaceId, addr: u64, size: u64) -> RegionDescriptor {
    RegionDescriptor {
        name: name.into(),
        space,
        addr,
        size,
    }
}

fn memory_with(bytes: &[u8]) -> GuestMemory {
    let mut mem = GuestMemory::new(bytes.len().max(1));
    let range = ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
    mem
}

fn boot_only(bytes: &[u8]) -> SpaceSnapshots {
    SpaceSnapshots::from([(AddressSpaceId::BOOT, memory_with(bytes))])
}

#[test]
fn extract_returns_named_region_within_bounds() {
    let spaces = boot_only(&[1u8, 2, 3, 4, 5, 6, 7, 8]);
    let extracted = extract_regions(&spaces, &[desc("head", AddressSpaceId::BOOT, 0, 4)]);
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].name, "head");
    assert_eq!(extracted[0].addr, 0);
    assert_eq!(extracted[0].data, vec![1, 2, 3, 4]);
}

#[test]
fn extract_zero_fills_when_addr_is_out_of_bounds() {
    let spaces = boot_only(&[0u8; 8]);
    let extracted = extract_regions(&spaces, &[desc("oob", AddressSpaceId::BOOT, 999_999, 16)]);
    assert_eq!(extracted[0].data, vec![0u8; 16]);
}

#[test]
fn extract_zero_fills_when_end_exceeds_memory() {
    let spaces = boot_only(&[0xAA; 4]);
    let extracted = extract_regions(&spaces, &[desc("straddle", AddressSpaceId::BOOT, 2, 8)]);
    assert_eq!(extracted[0].data, vec![0u8; 8]);
}

#[test]
fn extract_returns_empty_when_no_regions_requested() {
    let spaces = boot_only(&[0u8; 32]);
    assert!(extract_regions(&spaces, &[]).is_empty());
}

#[test]
fn a_child_space_region_reads_the_child_not_the_boot_space() {
    let child = AddressSpaceId::new(1);
    let spaces = SpaceSnapshots::from([
        (AddressSpaceId::BOOT, memory_with(&[0x11; 8])),
        (child, memory_with(&[0x22; 8])),
    ]);
    let extracted = extract_regions(
        &spaces,
        &[
            desc("boot", AddressSpaceId::BOOT, 0, 4),
            desc("child", child, 0, 4),
        ],
    );
    assert_eq!(extracted[0].data, vec![0x11; 4]);
    assert_eq!(extracted[1].data, vec![0x22; 4]);
}

#[test]
fn a_region_in_a_space_the_run_never_created_is_zero_filled() {
    let spaces = boot_only(&[0x11; 8]);
    let extracted = extract_regions(&spaces, &[desc("ghost", AddressSpaceId::new(3), 0, 4)]);
    assert_eq!(extracted[0].data, vec![0u8; 4]);
}

#[test]
fn an_auxiliary_region_above_the_flat_range_is_readable() {
    let mut mem = GuestMemory::new(0x100);
    mem.install_region(0xD000_0000, 0x1000, "stack", PageSize::Page4K)
        .unwrap();
    let range = ByteRange::new(GuestAddr::new(0xD000_0FF0), 4).unwrap();
    mem.apply_commit(range, &[9, 8, 7, 6]).unwrap();
    let spaces = SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)]);
    let extracted = extract_regions(
        &spaces,
        &[desc("stack_tail", AddressSpaceId::BOOT, 0xD000_0FF0, 4)],
    );
    assert_eq!(extracted[0].data, vec![9, 8, 7, 6]);
}

#[test]
fn a_region_straddling_two_mapped_regions_is_zero_filled() {
    let mut mem = GuestMemory::new(0x10);
    mem.install_region(0x10, 0x10, "aux", PageSize::Page4K)
        .unwrap();
    let low = ByteRange::new(GuestAddr::new(0), 0x10).unwrap();
    mem.apply_commit(low, &[0xAA; 0x10]).unwrap();
    let high = ByteRange::new(GuestAddr::new(0x10), 0x10).unwrap();
    mem.apply_commit(high, &[0xBB; 0x10]).unwrap();
    let spaces = SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)]);
    let extracted = extract_regions(&spaces, &[desc("boundary", AddressSpaceId::BOOT, 0xC, 8)]);
    assert_eq!(extracted[0].data, vec![0u8; 8]);
}

#[test]
fn a_region_whose_end_overflows_the_address_space_is_zero_filled() {
    let spaces = boot_only(&[0x11; 8]);
    let extracted = extract_regions(
        &spaces,
        &[desc("wrap", AddressSpaceId::BOOT, u64::MAX - 1, 4)],
    );
    assert_eq!(extracted[0].addr, u64::MAX - 1);
    assert_eq!(extracted[0].data, vec![0u8; 4]);
}

#[test]
fn a_zero_length_region_yields_no_bytes_wherever_it_points() {
    let spaces = boot_only(&[0x11; 8]);
    let extracted = extract_regions(
        &spaces,
        &[
            desc("inside", AddressSpaceId::BOOT, 4, 0),
            desc("outside", AddressSpaceId::BOOT, 999_999, 0),
        ],
    );
    assert!(extracted[0].data.is_empty());
    assert!(extracted[1].data.is_empty());
}

#[test]
fn a_reserved_strict_region_is_zero_filled_rather_than_faulting() {
    let mem = GuestMemory::from_regions(vec![Region::with_access(
        0x1000,
        0x10,
        "strict",
        PageSize::Page4K,
        RegionAccess::ReservedStrict,
    )])
    .unwrap();
    let spaces = SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)]);
    let extracted = extract_regions(&spaces, &[desc("strict", AddressSpaceId::BOOT, 0x1000, 8)]);
    assert_eq!(extracted[0].data, vec![0u8; 8]);
}

#[test]
fn a_reserved_zero_readable_region_reads_zeros_and_counts_the_provisional_read() {
    let mem = GuestMemory::from_regions(vec![Region::with_access(
        0x1000,
        0x10,
        "reserved",
        PageSize::Page4K,
        RegionAccess::ReservedZeroReadable,
    )])
    .unwrap();
    let spaces = SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)]);
    let extracted = extract_regions(&spaces, &[desc("rsx", AddressSpaceId::BOOT, 0x1000, 8)]);
    assert_eq!(extracted[0].data, vec![0u8; 8]);
    assert_eq!(spaces[&AddressSpaceId::BOOT].provisional_read_count(), 1);
}
