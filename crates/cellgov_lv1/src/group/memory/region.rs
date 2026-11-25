//! Hypercall 0, `allocate_memory`, and the memory regions the hypervisor
//! keeps for the partition.
//!
//! The hypervisor keeps each region it creates and the partition's region
//! counter. Only `allocate_memory` reads them.

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::ALLOCATE_MEMORY;
use cellgov_ps3_abi::lv1::memory::{
    allocate_memory_checks_read, game_os_region_counter_at_boot, region_address,
    ALLOCATE_MEMORY_FLAGS_MASK, ALLOCATE_MEMORY_FLAGS_R3, ALLOCATE_MEMORY_MAX_SIZE,
    ALLOCATE_MEMORY_PAGE_SIZE_EXP_MAX, ALLOCATE_MEMORY_PAGE_SIZE_EXP_MIN,
    ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3, ALLOCATE_MEMORY_R5_MAX, ALLOCATE_MEMORY_R5_R3,
    ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3, ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3, ALLOCATE_MEMORY_UNIT_ID,
};
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_ps3_abi::lv1::repository::{field, first_field, lookup, RepositoryKey};

use super::{REGION_COUNTER_SOURCE, REGION_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: ALLOCATE_MEMORY,
    name: "allocate_memory",
    cost: None,
    handler: |request, input| allocate(request, input.firmware, input.groups.memory().regions()),
}];

/// The page size exponents that the 4.93 kernel's boot allocations pass,
/// ascending. Its `sc` [OFW-4.93 lv2_kernel a:0x80000000002653c0] passes
/// 20, and its `sc` [OFW-4.93 lv2_kernel a:0x800000000026541c] passes 16.
const KERNEL_PAGE_SIZE_EXPS: &[u64] = &[16, 20];

/// One memory region that the hypervisor created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    /// The logical partition address, which the answer put in r4.
    pub lpar_address: u64,
    /// The size in bytes.
    pub size: u64,
    /// The page size exponent: each page holds 2^`page_size_exp` bytes.
    pub page_size_exp: u8,
    /// The memory unit id, which the answer put in r5.
    pub unit_id: u64,
    /// The value of the partition's region counter that the region took.
    pub counter: u64,
}

/// Fields 1 to 5: the address, the size, the page size exponent, the
/// unit id and the counter value.
impl LaneValue for MemoryRegion {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.lpar_address);
        lanes.lane(2, 0, self.size);
        lanes.lane(3, 0, u64::from(self.page_size_exp));
        lanes.lane(4, 0, self.unit_id);
        lanes.lane(5, 0, self.counter);
    }
}

/// The partition's memory regions and its region counter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryState {
    next_counter: Option<u64>,
    regions: Vec<MemoryRegion>,
}

impl MemoryState {
    /// Each region, in the order the hypervisor created them.
    pub fn regions(&self) -> &[MemoryRegion] {
        &self.regions
    }

    /// The counter value of the next region, or `None` before the first
    /// region.
    pub fn next_counter(&self) -> Option<u64> {
        self.next_counter
    }

    /// Keeps `region` and moves the counter past the value it took.
    pub(crate) fn create(&mut self, region: MemoryRegion) {
        self.take_counter(region.counter);
        self.regions.push(region);
    }

    /// Moves the counter past `counter`, the value that a region or a
    /// page table mapping took.
    pub(crate) fn take_counter(&mut self, counter: u64) {
        self.next_counter = Some(counter.saturating_add(1));
    }

    /// The sync-state terms of the regions and the counter.
    ///
    /// Each region is one object of the lane source `REGION_SOURCE`, keyed
    /// by its creation index. The counter is object 0 of
    /// the lane source `REGION_COUNTER_SOURCE`. A state with no region adds
    /// no term.
    pub fn sync_term(&self) -> u128 {
        let mut sum = match self.next_counter {
            Some(counter) => lanes::value_term(REGION_COUNTER_SOURCE, 0, &counter),
            None => 0,
        };
        for (index, region) in (0u64..).zip(&self.regions) {
            sum = sum.wrapping_add(lanes::value_term(REGION_SOURCE, index, region));
        }
        sum
    }

    fn allocated(&self) -> u64 {
        self.regions
            .iter()
            .fold(0u64, |sum, r| sum.saturating_add(r.size))
    }
}

/// The GameOS partition's memory past its real mode region at the
/// firmware version `firmware`: `bi.rgntotal` less the real mode size.
///
/// Linux `ps3_mm_init` asks lv1 for this amount for its one high
/// region. It is `None` when no repository source gives either value.
fn partition_pool(firmware: &str) -> Option<u64> {
    let bi = first_field(b"bi", 0);
    let total = game_os_node(bi, field(b"rgntotal", 0), 0, 0, firmware)?;
    total.checked_sub(real_mode_size(firmware)?)
}

/// The first value of the GameOS partition's repository node with the
/// key words `n1` to `n4` at the firmware version `firmware`.
fn game_os_node(n1: u64, n2: u64, n3: u64, n4: u64, firmware: &str) -> Option<u64> {
    let key = RepositoryKey {
        lpar: LPAR_ID_GAME_OS,
        n1,
        n2,
        n3,
        n4,
    };
    lookup(&key, firmware).map(|node| node.v1)
}

/// The size of the GameOS partition's real mode region at the firmware
/// version `firmware`: `bi.pu.rm_size`, or `None` when no repository
/// source gives it.
pub(crate) fn real_mode_size(firmware: &str) -> Option<u64> {
    let bi = first_field(b"bi", 0);
    let pu = game_os_node(bi, field(b"pu", 0), 0, 0, firmware)?;
    game_os_node(bi, field(b"pu", 0), pu, field(b"rm_size", 0), firmware)
}

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: ALLOCATE_MEMORY,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// The r3 that lv1's `allocate_memory` worker returns for the first of
/// its argument checks that the call fails, or `None` when every check
/// passes.
///
/// The arguments are the size in r3, the page size exponent in r4, r5
/// and the flags in r6. The checks run in lv1's order; see
/// [`ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3`].
fn lv1_argument_check(size: u64, exp: u64, r5: u64, flags: u64) -> Option<u64> {
    let exp_span = ALLOCATE_MEMORY_PAGE_SIZE_EXP_MAX - ALLOCATE_MEMORY_PAGE_SIZE_EXP_MIN;
    if size > ALLOCATE_MEMORY_MAX_SIZE {
        Some(ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3)
    } else if exp.wrapping_sub(ALLOCATE_MEMORY_PAGE_SIZE_EXP_MIN) > exp_span {
        Some(ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3)
    } else if size & ((1u64 << exp) - 1) != 0 {
        Some(ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3)
    } else if r5 > ALLOCATE_MEMORY_R5_MAX {
        Some(ALLOCATE_MEMORY_R5_R3)
    } else if flags & !ALLOCATE_MEMORY_FLAGS_MASK != 0 {
        Some(ALLOCATE_MEMORY_FLAGS_R3)
    } else {
        None
    }
}

/// Hypercall 0: a new region of r3 bytes in 2^r4-byte pages, with its
/// logical partition address in r4 and its memory unit id in r5.
///
/// A call that fails one of lv1's argument checks gets lv1's own r3,
/// with r4 to r10 cleared, at each firmware whose lv1 shows the checks.
/// This assumes that lv1 writes no return slot on that path.
///
/// The address takes the partition's region counter, which starts at the
/// firmware's boot value. The answer carries the region, and the
/// hypervisor keeps it. The call is a refusal in these cases:
/// - no lv1 image read shows the argument checks for `firmware`;
/// - r5 is 1, or r6 sets one of its low 4 bits, which lv1's checks pass;
/// - r4 is a page size exponent that the kernel's boot allocations do
///   not pass;
/// - r3 is 0;
/// - no source gives the boot counter or the partition's memory for
///   `firmware`;
/// - the regions would hold more than the partition's memory;
/// - the address encoding cannot hold the region.
pub(crate) fn allocate(
    request: &HypercallRequest,
    firmware: Option<&str>,
    memory: &MemoryState,
) -> HypercallDisposition {
    let (size, exp) = (request.r(3), request.r(4));
    let Some(firmware) = firmware else {
        return refuse(request, "an allocation with no firmware version");
    };
    if !allocate_memory_checks_read(firmware) {
        return refuse(
            request,
            "an allocation at a firmware whose lv1 argument checks are not read",
        );
    }
    if let Some(r3) = lv1_argument_check(size, exp, request.r(5), request.r(6)) {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []));
    }
    if request.r(5) != 0 {
        return refuse(request, "an allocation with r5 = 1");
    }
    if request.r(6) != 0 {
        return refuse(request, "an allocation with flags in the low 4 bits of r6");
    }
    let page_size_exp = match u8::try_from(exp) {
        Ok(page_size_exp) if KERNEL_PAGE_SIZE_EXPS.contains(&exp) => page_size_exp,
        _ => {
            return refuse(
                request,
                "an allocation page size that no kernel allocation is shown to pass",
            )
        }
    };
    if size == 0 {
        return refuse(request, "an allocation of size 0");
    }
    let (Some(boot_counter), Some(pool)) = (
        game_os_region_counter_at_boot(firmware),
        partition_pool(firmware),
    ) else {
        return refuse(
            request,
            "an allocation at a firmware whose region counter or partition memory no source gives",
        );
    };
    if memory.allocated().saturating_add(size) > pool {
        return refuse(request, "an allocation past the partition's memory");
    }
    let counter = memory.next_counter().unwrap_or(boot_counter);
    let Some(lpar_address) = region_address(size, counter) else {
        return refuse(
            request,
            "an allocation whose region address the encoding cannot hold",
        );
    };
    let mut answer = HypercallAnswer::success([lpar_address, ALLOCATE_MEMORY_UNIT_ID]);
    answer.push_change(MemoryRegion {
        lpar_address,
        size,
        page_size_exp,
        unit_id: ALLOCATE_MEMORY_UNIT_ID,
        counter,
    });
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/memory_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/memory_check_tests.rs"]
mod check_tests;
