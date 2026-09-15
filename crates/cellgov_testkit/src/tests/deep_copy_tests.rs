use super::*;
use crate::world::WritingUnit;
use cellgov_mem::PageSize;
use cellgov_time::Budget;

const CHILD: AddressSpaceId = AddressSpaceId::new(1);

fn two_space_fixture() -> ScenarioFixture {
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(10)
        .register(|rt: &mut Runtime| {
            let child = GuestMemory::from_regions(vec![
                Region::new(0, 16, "child_main", PageSize::Page4K),
                Region::with_access(
                    0x1000,
                    16,
                    "zero_readable",
                    PageSize::Page4K,
                    RegionAccess::ReservedZeroReadable,
                ),
                Region::with_access(
                    0x2000,
                    16,
                    "strict",
                    PageSize::Page4K,
                    RegionAccess::ReservedStrict,
                ),
            ])
            .unwrap();
            rt.create_address_space_with(CHILD, child).unwrap();
            let seed = ByteRange::new(GuestAddr::new(0), 4).unwrap();
            rt.place_bytes(CHILD, seed, &[1, 2, 3, 4]).unwrap();
            rt.register_unit_with(|id| WritingUnit::at_zero(id, 2));
        })
        .build()
}

fn head(mem: &GuestMemory, addr: u64) -> Option<Vec<u8>> {
    mem.read(ByteRange::new(GuestAddr::new(addr), 4).unwrap())
        .map(<[u8]>::to_vec)
}

#[test]
fn the_boot_copy_holds_the_last_committed_write() {
    let result = run(two_space_fixture());
    let boot = &result.final_spaces[&AddressSpaceId::BOOT];
    assert_eq!(head(boot, 0), Some(vec![2, 2, 2, 2]));
    assert_eq!(&result.final_memory[..4], &[2, 2, 2, 2]);
}

#[test]
fn a_child_copy_keeps_every_region_access_mode() {
    let result = run(two_space_fixture());
    let child = &result.final_spaces[&CHILD];
    let modes: Vec<(&str, RegionAccess)> =
        child.regions().map(|r| (r.label(), r.access())).collect();
    assert_eq!(
        modes,
        vec![
            ("child_main", RegionAccess::ReadWrite),
            ("zero_readable", RegionAccess::ReservedZeroReadable),
            ("strict", RegionAccess::ReservedStrict),
        ]
    );
    assert_eq!(head(child, 0), Some(vec![1, 2, 3, 4]));
    assert_eq!(head(child, 0x1000), Some(vec![0; 4]));
    assert_eq!(head(child, 0x2000), None);
}

#[test]
fn a_single_space_copy_hashes_like_the_runtime_memory() {
    let result = run(ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(10)
        .register(|rt: &mut Runtime| {
            rt.register_unit_with(|id| WritingUnit::at_zero(id, 3));
        })
        .build());
    assert_eq!(result.final_spaces.len(), 1);
    let boot = &result.final_spaces[&AddressSpaceId::BOOT];
    assert_eq!(boot.content_hash(), result.final_memory_hash.raw());
}

#[test]
fn a_pooled_rerun_resets_while_an_earlier_result_is_still_held() {
    let mut pool = MemoryPool::new();
    let first = run_pooled(two_space_fixture(), &mut pool);
    let second = run_pooled(two_space_fixture(), &mut pool);
    let boot = &first.final_spaces[&AddressSpaceId::BOOT];
    assert_eq!(head(boot, 0), Some(vec![2, 2, 2, 2]));
    assert_eq!(first.final_memory_hash, second.final_memory_hash);
}
