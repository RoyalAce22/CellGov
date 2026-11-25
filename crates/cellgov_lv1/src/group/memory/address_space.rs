//! Hypercalls 2 and 7, `construct_virtual_address_space` and
//! `select_virtual_address_space`, and the virtual address spaces the
//! hypervisor keeps for the partition.
//!
//! The hypervisor keeps each address space it constructs and the vas id
//! that the partition's PPE selected. These two hypercalls and the page
//! table calls read them.

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::address_space::{
    address_space_handlers, game_os_boot_vas_id, htab_size, htab_size_exp, page_shifts_accepted,
    CONSTRUCT_VAS_HTAB_SIZE_EXP_R3, CONSTRUCT_VAS_PAGE_SHIFT_R3, CONSTRUCT_VAS_PAGE_SIZE_COUNT_R3,
    HTAB_SIZE_EXP_MIN, PAGE_SIZE_COUNT_MAX,
};
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_VIRTUAL_ADDRESS_SPACE, SELECT_VIRTUAL_ADDRESS_SPACE};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use cellgov_mem::GuestMemory;

use super::{page_table, ADDRESS_SPACE_SOURCE, SELECTED_ADDRESS_SPACE_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::{CallerTranslation, HypercallRequest};

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[
    Hypercall {
        number: CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
        name: "construct_virtual_address_space",
        cost: None,
        handler: |request, input| {
            construct(request, input.firmware, input.groups.memory().spaces())
        },
    },
    Hypercall {
        number: SELECT_VIRTUAL_ADDRESS_SPACE,
        name: "select_virtual_address_space",
        cost: None,
        handler: |request, input| {
            select(
                request,
                input.caller,
                input.firmware,
                input.groups.memory().spaces(),
                input.storage,
            )
        },
    },
];

/// The page size count that the 4.93 kernel's `sc`
/// [OFW-4.93 lv2_kernel a:0x800000000005dcf4] passes in r4.
const KERNEL_PAGE_SIZE_COUNT: u64 = 2;

/// The page shifts that the 4.93 kernel's `sc`
/// [OFW-4.93 lv2_kernel a:0x800000000005dcf4] passes in r5: 16 and 20.
const KERNEL_PAGE_SHIFTS: u64 = 0x1014_0000_0000_0000;

/// One virtual address space that the hypervisor constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpace {
    /// The vas id, which the answer put in r4.
    pub vas_id: u64,
    /// The page table size exponent that lv1 uses: the page table holds
    /// 2^`htab_size_exp` bytes.
    pub htab_size_exp: u64,
    /// The page size count, from r4.
    pub page_size_count: u64,
    /// The page shifts, from r5.
    pub page_shifts: u64,
    /// The logical partition address of the page table's mapping, or
    /// `None` before `map_htab` maps it.
    pub mapping: Option<u64>,
}

/// Fields 1 to 4: the vas id, the page table size exponent, the page
/// size count and the page shifts. Field 5, present only for a mapped
/// page table, is the mapping's address.
impl LaneValue for AddressSpace {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.vas_id);
        lanes.lane(2, 0, self.htab_size_exp);
        lanes.lane(3, 0, self.page_size_count);
        lanes.lane(4, 0, self.page_shifts);
        if let Some(mapping) = self.mapping {
            lanes.lane(5, 0, mapping);
        }
    }
}

/// A change to the address spaces that an answer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceChange {
    /// The call constructed this address space.
    Constructed(AddressSpace),
    /// The call selected the address space with this vas id.
    Selected(u64),
    /// The call mapped the page table of an address space.
    Mapped {
        /// The vas id of the address space.
        vas_id: u64,
        /// The mapping's logical partition address.
        lpar_address: u64,
        /// The value of the partition's region counter that the mapping
        /// took.
        counter: u64,
    },
}

/// The partition's virtual address spaces and the selected one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddressSpaceState {
    spaces: Vec<AddressSpace>,
    selected: Option<u64>,
}

impl AddressSpaceState {
    /// Each address space, in the order the hypervisor constructed them.
    pub fn spaces(&self) -> &[AddressSpace] {
        &self.spaces
    }

    /// The vas id that the partition's PPE selected, or `None` before
    /// the first selection.
    pub fn selected(&self) -> Option<u64> {
        self.selected
    }

    /// Applies `change`.
    pub(crate) fn apply(&mut self, change: AddressSpaceChange) {
        match change {
            AddressSpaceChange::Constructed(space) => self.spaces.push(space),
            AddressSpaceChange::Selected(vas_id) => self.selected = Some(vas_id),
            AddressSpaceChange::Mapped {
                vas_id,
                lpar_address,
                ..
            } => {
                if let Some(space) = self.spaces.iter_mut().find(|s| s.vas_id == vas_id) {
                    space.mapping = Some(lpar_address);
                }
            }
        }
    }

    /// The sync-state terms of the address spaces and the selection.
    ///
    /// Each address space is one object of
    /// the lane source `ADDRESS_SPACE_SOURCE`, keyed by its construction
    /// index. The selected vas id is object 0 of
    /// the lane source `SELECTED_ADDRESS_SPACE_SOURCE`. A state with no
    /// address space and no selection adds no term.
    pub fn sync_term(&self) -> u128 {
        let mut sum = match self.selected {
            Some(vas_id) => lanes::value_term(SELECTED_ADDRESS_SPACE_SOURCE, 0, &vas_id),
            None => 0,
        };
        for (index, space) in (0u64..).zip(&self.spaces) {
            sum = sum.wrapping_add(lanes::value_term(ADDRESS_SPACE_SOURCE, index, space));
        }
        sum
    }

    /// The address space with the vas id `vas_id`, or `None` when the
    /// hypervisor did not construct it.
    pub fn space(&self, vas_id: u64) -> Option<&AddressSpace> {
        self.spaces.iter().find(|space| space.vas_id == vas_id)
    }
}

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// The r3 of the first lv1 argument check that the call fails, or `None` when all pass.
///
/// The arguments are the page table size exponent in r3, the page size
/// count in r4 and the page shifts in r5. The checks run in the order of
/// lv1's `construct_virtual_address_space` handler; see
/// [`CONSTRUCT_VAS_HTAB_SIZE_EXP_R3`].
fn lv1_argument_check(exp: u64, count: u64, shifts: u64) -> Option<u64> {
    if htab_size_exp(exp).is_none() {
        Some(CONSTRUCT_VAS_HTAB_SIZE_EXP_R3)
    } else if count > PAGE_SIZE_COUNT_MAX {
        Some(CONSTRUCT_VAS_PAGE_SIZE_COUNT_R3)
    } else if !page_shifts_accepted(count, shifts) {
        Some(CONSTRUCT_VAS_PAGE_SHIFT_R3)
    } else {
        None
    }
}

/// Hypercall 2: a new virtual address space.
///
/// A call that fails one of lv1's argument checks gets lv1's own r3,
/// with r4 to r10 cleared. A call that passes them gets the vas id in
/// r4, the page table size in bytes in r5 and r6 to r10 cleared. The
/// answer carries the address space, and the hypervisor keeps it.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the call passes lv1's checks with a page table size or page sizes
///   that the kernel's call does not pass;
/// - the hypervisor already constructed an address space, so no source
///   gives the vas id;
/// - no source gives the boot vas id for `firmware`.
pub(crate) fn construct(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spaces: &AddressSpaceState,
) -> HypercallDisposition {
    let (exp, count, shifts) = (request.r(3), request.r(4), request.r(5));
    let Some(firmware) = firmware.filter(|fw| address_space_handlers(fw).is_some()) else {
        return refuse(
            request,
            "an address space construction at a firmware whose lv1 handler is not read",
        );
    };
    if let Some(r3) = lv1_argument_check(exp, count, shifts) {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []));
    }
    let htab_size_exp = match htab_size_exp(exp) {
        Some(HTAB_SIZE_EXP_MIN) => HTAB_SIZE_EXP_MIN,
        _ => {
            return refuse(
                request,
                "a page table size that no kernel call is shown to pass",
            )
        }
    };
    if (count, shifts) != (KERNEL_PAGE_SIZE_COUNT, KERNEL_PAGE_SHIFTS) {
        return refuse(request, "page sizes that no kernel call is shown to pass");
    }
    if !spaces.spaces().is_empty() {
        return refuse(
            request,
            "a second address space construction, whose vas id no source gives",
        );
    }
    let Some(vas_id) = game_os_boot_vas_id(firmware) else {
        return refuse(
            request,
            "an address space construction at a firmware whose boot vas id no source gives",
        );
    };
    let outputs = [vas_id, htab_size(htab_size_exp)];
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), outputs);
    answer.push_change(AddressSpaceChange::Constructed(AddressSpace {
        vas_id,
        htab_size_exp,
        page_size_count: count,
        page_shifts: shifts,
        mapping: None,
    }));
    HypercallDisposition::Answered(answer)
}

/// Hypercall 7: selects the address space whose vas id is in r3, with
/// r4 to r10 cleared.
///
/// The calling thread loads SDR1 with the address space's page table in
/// `storage`, and HID6\[LB\] with its page sizes; see
/// [`cellgov_ps3_abi::lv1::page_table::select_sdr1`].
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - `caller` refuses the page table load;
/// - r3 is 0, which lv1 answers by selecting the partition's default
///   address space, and CellGov does not model that address space;
/// - the hypervisor did not construct the address space;
/// - `storage` does not hold the page table.
pub(crate) fn select(
    request: &HypercallRequest,
    caller: CallerTranslation,
    firmware: Option<&str>,
    spaces: &AddressSpaceState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    let vas_id = request.r(3);
    if firmware.and_then(address_space_handlers).is_none() {
        return refuse(
            request,
            "an address space selection at a firmware whose lv1 handler is not read",
        );
    }
    if caller == CallerTranslation::Refuses {
        return refuse(
            request,
            "a selection from a unit that holds no SDR1 or HID6 for the page table load",
        );
    }
    if vas_id == 0 {
        return refuse(
            request,
            "a selection of the partition's default address space",
        );
    }
    let Some(space) = spaces.space(vas_id) else {
        return refuse(
            request,
            "a selection of an address space that this hypervisor did not construct",
        );
    };
    if !page_table::holds_table(storage, space) {
        return refuse(
            request,
            "a selection of an address space with no storage for its page table",
        );
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(AddressSpaceChange::Selected(vas_id));
    answer.page_table = Some(page_table::load(space));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/address_space_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/address_space_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/select_load_tests.rs"]
mod select_load_tests;

#[cfg(test)]
#[path = "tests/select_caller_tests.rs"]
mod select_caller_tests;
