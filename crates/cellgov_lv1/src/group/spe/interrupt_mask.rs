//! Hypercall 61, `set_spe_interrupt_mask`, and the interrupt mask that the
//! hypervisor keeps for each class of each logical SPE.
//!
//! lv1 keeps the mask at +112 of the interrupt object of the class's irq
//! outlet; see [`cellgov_ps3_abi::lv1::spe`]. So the hypervisor keeps the
//! mask of each class of each SPE, and hashes it.

use std::collections::BTreeMap;

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::SET_SPE_INTERRUPT_MASK;
use cellgov_ps3_abi::lv1::spe::{
    spe_interrupt_mask_handler, LOGICAL_SPE_TYPE_1, SET_SPE_INTERRUPT_MASK_CLASS_R3,
    SPE_INTERRUPT_CLASS_MASKS,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::construct::LogicalSpeState;
use super::{SpeChange, LOGICAL_SPE_INTERRUPT_MASK_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: SET_SPE_INTERRUPT_MASK,
    name: "set_spe_interrupt_mask",
    cost: None,
    handler: |request, input| {
        set_interrupt_mask(request, input.firmware, input.groups.spe().spes())
    },
}];

/// The interrupt mask of one interrupt class of one logical SPE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptMask {
    /// The spe id.
    pub spe: u64,
    /// The interrupt class, 0 to 2.
    pub class: u64,
    /// The mask, which lv1 keeps at +112 of the class's interrupt object.
    pub mask: u64,
}

/// Fields 1 to 3: the spe id, the class and the mask.
impl LaneValue for InterruptMask {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.spe);
        lanes.lane(2, 0, self.class);
        lanes.lane(3, 0, self.mask);
    }
}

/// The interrupt mask of each class of each logical SPE, by spe id and
/// class.
///
/// A class with no entry has the mask 0, as the interrupt object's
/// constructor leaves it. A mask of 0 removes the entry, so that the state
/// of a class with the mask 0 is one state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterruptMaskState {
    masks: BTreeMap<(u64, u64), InterruptMask>,
}

impl InterruptMaskState {
    /// Each mask that is not 0, by its spe id and class.
    pub fn masks(&self) -> &BTreeMap<(u64, u64), InterruptMask> {
        &self.masks
    }

    /// Keeps `mask` in place of the mask of its SPE and class.
    pub(crate) fn set(&mut self, mask: InterruptMask) {
        if mask.mask == 0 {
            self.masks.remove(&(mask.spe, mask.class));
        } else {
            self.masks.insert((mask.spe, mask.class), mask);
        }
    }

    /// The sync-state terms of the masks.
    ///
    /// Each mask is one object of the lane source
    /// `LOGICAL_SPE_INTERRUPT_MASK_SOURCE`, keyed by its index in spe id
    /// and class order. A state with no mask adds no term.
    pub fn sync_term(&self) -> u128 {
        (0u64..)
            .zip(self.masks.values())
            .fold(0u128, |sum, (index, mask)| {
                sum.wrapping_add(lanes::value_term(
                    LOGICAL_SPE_INTERRUPT_MASK_SOURCE,
                    index,
                    mask,
                ))
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

/// Hypercall 61: the class r4 of the SPE r3 keeps the mask r5, with r4 to
/// r10 cleared.
///
/// The answers:
/// - r4 is above 2: [`SET_SPE_INTERRUPT_MASK_CLASS_R3`], and no mask
///   changes. lv1 does not read r3 or r5 on this path.
/// - otherwise: success, and the class keeps r5 AND the class's entry of
///   [`SPE_INTERRUPT_CLASS_MASKS`].
///
/// No call that CellGov answers changes the doubleword at +184 of an SPE,
/// or gives an SPE a physical SPU. So each SPE passes the check of lv1's
/// call function, and lv1's mask method stores the mask and returns 0.
///
/// The partition's object table holds objects that the hypervisor does
/// not keep. So the call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 names no SPE that the hypervisor holds;
/// - the SPE is not type 1, whose interrupt objects no read gives.
pub(crate) fn set_interrupt_mask(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spes: &LogicalSpeState,
) -> HypercallDisposition {
    if firmware.is_none_or(|fw| spe_interrupt_mask_handler(fw).is_none()) {
        return refuse(
            request,
            "an SPE interrupt mask call at a firmware whose lv1 handler is not read",
        );
    }
    let class = request.r(4);
    let Some(&class_mask) = usize::try_from(class)
        .ok()
        .and_then(|index| SPE_INTERRUPT_CLASS_MASKS.get(index))
    else {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            SET_SPE_INTERRUPT_MASK_CLASS_R3,
            [],
        ));
    };
    let Some(spe) = spes.spe(request.r(3)) else {
        return refuse(
            request,
            "an SPE interrupt mask call on an spe id that the hypervisor did not construct",
        );
    };
    if spe.spe_type != LOGICAL_SPE_TYPE_1 {
        return refuse(
            request,
            "an SPE interrupt mask call on an SPE whose type's interrupt objects no read gives",
        );
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(SpeChange::InterruptMaskSet(InterruptMask {
        spe: spe.id,
        class,
        mask: request.r(5) & class_mask,
    }));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/interrupt_mask_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/interrupt_mask_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/interrupt_mask_boot_tests.rs"]
mod boot_tests;
