use super::*;
use crate::state::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::memory::{game_os_region_counter_at_boot, region_address};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's first boot allocation
/// [OFW-4.93 lv2_kernel a:0x80000000002653c0].
const PC: u64 = 0x8000_0000_0026_53c0;

fn request(inputs: [u64; 4]) -> HypercallRequest {
    let mut all = [0; 8];
    all[..4].copy_from_slice(&inputs);
    HypercallRequest {
        unit: UnitId::new(1),
        number: ALLOCATE_MEMORY,
        inputs: all,
        pc: PC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(7),
    }
}

/// The first value of the GameOS node `bi.<n2>[.<n3>.<n4>]` at 4.93,
/// from the console capture rows of the repository table.
fn bi_4_93(n2: &[u8], n3: u64, n4: &[u8]) -> u64 {
    let key = RepositoryKey {
        lpar: LPAR_ID_GAME_OS,
        n1: first_field(b"bi", 0),
        n2: field(n2, 0),
        n3,
        n4: if n4.is_empty() { 0 } else { field(n4, 0) },
    };
    lookup(&key, "4.93").expect("a 4.93 row").v1
}

/// The sizes of the 4.93 kernel's two boot allocations, as its code
/// computes them from `bi.rgntotal` and `rm_size`
/// [OFW-4.93 lv2_kernel a:0x800000000000cca8-0x800000000000ccf8]
/// [OFW-4.93 lv2_kernel a:0x8000000000265368-0x800000000026541c].
fn kernel_sizes_4_93() -> (u64, u64) {
    let total = bi_4_93(b"rgntotal", 0, b"");
    let pu = bi_4_93(b"pu", 0, b"");
    let rm_size = bi_4_93(b"pu", pu, b"rm_size");
    let remainder = total & 0xf_ffff;
    (total - rm_size - remainder, remainder & !0xffff)
}

fn hypervisor_4_93() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    hv
}

fn refused(what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: ALLOCATE_MEMORY,
        unit: UnitId::new(1),
        pc: PC,
        what,
    })
}

#[test]
fn the_4_93_boot_allocations_answer_consecutive_counter_addresses_and_unit_id_1() {
    let (first, second) = kernel_sizes_4_93();
    let counter = game_os_region_counter_at_boot("4.93").expect("4.93 counter");
    let mut hv = hypervisor_4_93();
    let expected = |size: u64, exp: u8, counter: u64| {
        let lpar_address = region_address(size, counter).expect("an address");
        let mut answer = HypercallAnswer::success([lpar_address, ALLOCATE_MEMORY_UNIT_ID]);
        answer.push_change(MemoryRegion {
            lpar_address,
            size,
            page_size_exp: exp,
            unit_id: ALLOCATE_MEMORY_UNIT_ID,
            counter,
        });
        HypercallDisposition::Answered(answer)
    };
    assert_eq!(
        hv.dispatch(&request([first, 20, 0, 0])),
        expected(first, 20, counter)
    );
    assert_eq!(
        hv.dispatch(&request([second, 16, 0, 0])),
        expected(second, 16, counter + 1)
    );
    assert_eq!(hv.refusal(), None);
}

#[test]
fn the_hypervisor_keeps_each_region_in_order_and_the_counter_past_the_last() {
    let (first, second) = kernel_sizes_4_93();
    let counter = game_os_region_counter_at_boot("4.93").expect("4.93 counter");
    let mut hv = hypervisor_4_93();
    assert_eq!(hv.memory(), &MemoryState::default());
    hv.dispatch(&request([first, 20, 0, 0]));
    hv.dispatch(&request([second, 16, 0, 0]));
    let kept: Vec<(u64, u64, u8, u64)> = hv
        .memory()
        .regions()
        .iter()
        .map(|r| (r.lpar_address, r.size, r.page_size_exp, r.counter))
        .collect();
    assert_eq!(
        kept,
        vec![
            (region_address(first, counter).unwrap(), first, 20, counter),
            (
                region_address(second, counter + 1).unwrap(),
                second,
                16,
                counter + 1
            ),
        ]
    );
    assert_eq!(hv.memory().next_counter(), Some(counter + 2));
}

#[test]
fn an_allocation_past_the_partition_memory_is_refused_and_keeps_no_region() {
    let (first, second) = kernel_sizes_4_93();
    let mut hv = hypervisor_4_93();
    hv.dispatch(&request([first, 20, 0, 0]));
    hv.dispatch(&request([second, 16, 0, 0]));
    let before = hv.memory().clone();
    assert_eq!(
        hv.dispatch(&request([0x1_0000, 16, 0, 0])),
        refused("an allocation past the partition's memory")
    );
    assert_eq!(hv.memory(), &before);
    let mut fresh = hypervisor_4_93();
    assert_eq!(
        fresh.dispatch(&request([first + 0x10_0000, 20, 0, 0])),
        refused("an allocation past the partition's memory")
    );
    assert!(fresh.memory().regions().is_empty());
}

#[test]
fn an_allocation_with_r5_1_or_low_flags_is_refused() {
    let (first, _) = kernel_sizes_4_93();
    let mut hv = hypervisor_4_93();
    assert_eq!(
        hv.dispatch(&request([first, 20, 1, 0])),
        refused("an allocation with r5 = 1")
    );
    assert_eq!(
        hv.dispatch(&request([first, 20, 0, 4])),
        refused("an allocation with flags in the low 4 bits of r6")
    );
    assert!(hv.memory().regions().is_empty());
}

#[test]
fn an_allocation_page_size_in_lv1s_range_the_kernel_does_not_pass_is_refused() {
    let mut hv = hypervisor_4_93();
    for exp in [12, 15, 17, 19, 21, 24] {
        assert_eq!(
            hv.dispatch(&request([0x100_0000, exp, 0, 0])),
            refused("an allocation page size that no kernel allocation is shown to pass"),
            "{exp}"
        );
    }
    assert!(hv.memory().regions().is_empty());
}

#[test]
fn an_allocation_of_size_0_is_refused() {
    let mut hv = hypervisor_4_93();
    for exp in [16, 20] {
        assert_eq!(
            hv.dispatch(&request([0, exp, 0, 0])),
            refused("an allocation of size 0"),
            "2^{exp} pages"
        );
    }
    assert!(hv.memory().regions().is_empty());
}

#[test]
fn an_allocation_with_no_firmware_or_a_floor_firmware_is_refused() {
    let (first, _) = kernel_sizes_4_93();
    let mut none = Hypervisor::new();
    assert_eq!(
        none.dispatch(&request([first, 20, 0, 0])),
        refused("an allocation with no firmware version")
    );
    let mut at_1_50 = Hypervisor::new();
    at_1_50.set_firmware("1.50");
    assert_eq!(
        at_1_50.dispatch(&request([first, 20, 0, 0])),
        refused("an allocation at a firmware whose lv1 argument checks are not read")
    );
    for fw in ["1.94", "3.70"] {
        let mut hv = Hypervisor::new();
        hv.set_firmware(fw);
        assert_eq!(
            hv.dispatch(&request([first, 20, 0, 0])),
            refused(
                "an allocation at a firmware whose region counter or partition memory no source gives"
            ),
            "{fw}"
        );
        assert!(hv.memory().regions().is_empty(), "{fw}");
    }
}

#[test]
fn an_allocation_whose_counter_reaches_the_exponent_bits_is_refused() {
    let memory = MemoryState {
        next_counter: Some(1 << 22),
        regions: Vec::new(),
    };
    assert_eq!(
        allocate(&request([0x10_0000, 20, 0, 0]), Some("4.93"), &memory),
        refused("an allocation whose region address the encoding cannot hold")
    );
}
