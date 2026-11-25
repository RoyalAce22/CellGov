//! Hypercall 57, `construct_logical_spe`, and the logical SPEs that the
//! hypervisor keeps for the partition.
//!
//! lv1 gives a new SPE an id in the partition's object table and two
//! regions: an area and a shadow page; see [`cellgov_ps3_abi::lv1::spe`].
//! Later SPE calls look the SPE up by its id. So the hypervisor keeps
//! each SPE, and hashes it.
//!
//! A console capture gives the spe id of the GameOS kernel's boot call by
//! its `sc` address. So the handler reads [`HypercallRequest::pc`],
//! although lv1 does not read it.

use std::collections::BTreeMap;

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::CONSTRUCT_LOGICAL_SPE;
use cellgov_ps3_abi::lv1::memory::{game_os_region_counter_at_boot, region_address};
use cellgov_ps3_abi::lv1::spe::{
    construct_logical_spe_handler, game_os_spe_call, LOGICAL_SPE_AREA_SIZE,
    LOGICAL_SPE_PRIV2_OFFSET, LOGICAL_SPE_PROBLEM_OFFSET,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::shadow::shadow_page_real;
use super::{SpeChange, LOGICAL_SPE_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::memory::{MemoryChange, MemoryGroupState};
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: CONSTRUCT_LOGICAL_SPE,
    name: "construct_logical_spe",
    cost: None,
    handler: |request, input| {
        construct(
            request,
            input.firmware,
            input.groups.memory(),
            input.groups.spe().spes(),
        )
    },
}];

/// The page shift that the 4.93 kernel's boot call passes in each of r3
/// to r7 [OFW-4.93 lv2_kernel a:0x800000000004f158-0x800000000004f168].
const KERNEL_PAGE_SHIFT: u64 = 12;

/// Log2 of the size of each shadow page: the page shift in r7 of the
/// kernel's call, the only call that the construct answers.
pub(super) const SHADOW_PAGE_SHIFT: u32 = KERNEL_PAGE_SHIFT as u32;

/// One logical SPE that the hypervisor constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalSpe {
    /// The spe id in the partition's object table, which the answer put
    /// in r9.
    pub id: u64,
    /// The SPE type, from r9 of the call.
    pub spe_type: u64,
    /// The vas id, from r8 of the call.
    pub vas_id: u64,
    /// The logical partition address of the SPE's area. The local store
    /// is at the start of the area, and the answer put this address in
    /// r6.
    pub area: u64,
    /// The logical partition address of the shadow page, which the answer
    /// put in r8.
    pub shadow: u64,
    /// The real address of the shadow page in CellGov's shadow page
    /// storage.
    pub shadow_real: u64,
}

/// Fields 1 to 6: the spe id, the SPE type, the vas id, the area's
/// address, the shadow page's address and its real address.
impl LaneValue for LogicalSpe {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.id);
        lanes.lane(2, 0, self.spe_type);
        lanes.lane(3, 0, self.vas_id);
        lanes.lane(4, 0, self.area);
        lanes.lane(5, 0, self.shadow);
        lanes.lane(6, 0, self.shadow_real);
    }
}

/// The partition's logical SPEs, by spe id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogicalSpeState {
    spes: BTreeMap<u64, LogicalSpe>,
}

impl LogicalSpeState {
    /// Each SPE, by its spe id.
    pub fn spes(&self) -> &BTreeMap<u64, LogicalSpe> {
        &self.spes
    }

    /// The SPE with the spe id `id`, or `None` when the hypervisor did not
    /// construct it.
    pub fn spe(&self, id: u64) -> Option<&LogicalSpe> {
        self.spes.get(&id)
    }

    /// Keeps `spe`.
    pub(crate) fn construct(&mut self, spe: LogicalSpe) {
        self.spes.insert(spe.id, spe);
    }

    /// The sync-state terms of the SPEs.
    ///
    /// Each SPE is one object of the lane source `LOGICAL_SPE_SOURCE`,
    /// keyed by its index in spe id order. A state with no SPE adds no
    /// term.
    pub fn sync_term(&self) -> u128 {
        (0u64..)
            .zip(self.spes.values())
            .fold(0u128, |sum, (index, spe)| {
                sum.wrapping_add(lanes::value_term(LOGICAL_SPE_SOURCE, index, spe))
            })
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

/// Whether the call passes the inputs of the kernel's call: the kernel's
/// page shift in r3 to r7, and the SPE type `spe_type` in r9.
fn kernel_inputs(request: &HypercallRequest, spe_type: u64) -> bool {
    (3..=7).all(|reg| request.r(reg) == KERNEL_PAGE_SHIFT) && request.r(9) == spe_type
}

/// Hypercall 57: a new type 1 logical SPE.
///
/// The answer gives these outputs:
/// - r4 to r6: the priv2 area, the problem state area and the local
///   store, in the SPE's area;
/// - r7: 0;
/// - r8: the shadow page;
/// - r9: the spe id;
/// - r10: 0.
///
/// The area takes the partition's next region counter value, and the
/// shadow page takes the value after it. The shadow page gets the next
/// page of CellGov's shadow page storage. The answer carries the SPE and
/// the shadow page's counter value, and the hypervisor keeps both.
///
/// lv1 gives the SPE the first free id of the partition's object table.
/// The partition's other objects decide that id. So CellGov answers the
/// id that a console capture gives for the call's `sc` address.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the spe id for the call's address at `firmware`;
/// - the call passes other inputs than the kernel's call at that address;
/// - r8 names no address space that the hypervisor constructed;
/// - the hypervisor already holds an SPE with that id;
/// - the shadow page storage holds no page for one more SPE;
/// - no source gives the region counter for `firmware`;
/// - the address encoding cannot hold the area or the shadow page.
pub(crate) fn construct(
    request: &HypercallRequest,
    firmware: Option<&str>,
    memory: &MemoryGroupState,
    spes: &LogicalSpeState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| construct_logical_spe_handler(fw).is_some()) else {
        return refuse(
            request,
            "a logical SPE construct at a firmware whose lv1 handler is not read",
        );
    };
    let Some(call) = game_os_spe_call(firmware, request.pc) else {
        return refuse(
            request,
            "a logical SPE construct whose spe id no source gives for its address",
        );
    };
    if !kernel_inputs(request, call.spe_type) {
        return refuse(
            request,
            "a logical SPE construct with inputs that the kernel's call does not pass",
        );
    }
    let vas_id = request.r(8);
    if memory.spaces().space(vas_id).is_none() {
        return refuse(
            request,
            "a logical SPE construct on a vas id that the hypervisor did not construct",
        );
    }
    if spes.spe(call.id).is_some() {
        return refuse(
            request,
            "a logical SPE construct whose spe id the hypervisor already holds",
        );
    }
    let Some(shadow_real) = shadow_page_real(spes.spes().len() as u64) else {
        return refuse(
            request,
            "a logical SPE construct past CellGov's shadow page storage",
        );
    };
    let Some(counter) = memory
        .regions()
        .next_counter()
        .or_else(|| game_os_region_counter_at_boot(firmware))
    else {
        return refuse(
            request,
            "a logical SPE construct at a firmware whose region counter no source gives",
        );
    };
    let shadow_counter = counter.saturating_add(1);
    let (Some(area), Some(shadow)) = (
        region_address(LOGICAL_SPE_AREA_SIZE, counter),
        region_address(1 << SHADOW_PAGE_SHIFT, shadow_counter),
    ) else {
        return refuse(
            request,
            "a logical SPE construct whose region addresses the encoding cannot hold",
        );
    };
    let outputs = [
        area + LOGICAL_SPE_PRIV2_OFFSET,
        area + LOGICAL_SPE_PROBLEM_OFFSET,
        area,
        0,
        shadow,
        call.id,
    ];
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), outputs);
    answer.push_change(SpeChange::Constructed(LogicalSpe {
        id: call.id,
        spe_type: call.spe_type,
        vas_id,
        area,
        shadow,
        shadow_real,
    }));
    answer.push_change(MemoryChange::CounterTaken(shadow_counter));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/construct_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/construct_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/construct_capture_tests.rs"]
mod capture_tests;

#[cfg(test)]
#[path = "tests/construct_shadow_tests.rs"]
mod shadow_tests;
