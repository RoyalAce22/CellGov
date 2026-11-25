//! Hypercall 64, `set_spe_transition_notifier`, and the transition
//! notifier that the hypervisor keeps for each logical SPE.
//!
//! lv1 keeps the notifier in the SPE object: the event receive port at
//! +144 and the mask at +152; see [`cellgov_ps3_abi::lv1::spe`]. So the
//! hypervisor keeps the notifier of each SPE, and hashes it.
//!
//! lv1 looks up the port in the partition's object table. So the handler
//! reads the event receive ports that the interrupt group keeps.

use std::collections::BTreeMap;

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::SET_SPE_TRANSITION_NOTIFIER;
use cellgov_ps3_abi::lv1::spe::{
    spe_transition_notifier_handler, LOGICAL_SPE_TYPE_1, LOGICAL_SPE_TYPE_1_TRANSITION_MASK,
    SET_SPE_TRANSITION_NOTIFIER_MASK_R3,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::construct::LogicalSpeState;
use super::{SpeChange, LOGICAL_SPE_NOTIFIER_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::interrupt::event_port::EventReceivePortState;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: SET_SPE_TRANSITION_NOTIFIER,
    name: "set_spe_transition_notifier",
    cost: None,
    handler: |request, input| {
        set_notifier(
            request,
            input.firmware,
            input.groups.spe().spes(),
            input.groups.interrupt().ports(),
        )
    },
}];

/// The transition notifier of one logical SPE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionNotifier {
    /// The spe id.
    pub spe: u64,
    /// The id of the event receive port, which lv1 keeps at +144 of the
    /// SPE.
    pub port: u64,
    /// The mask, which lv1 keeps at +152 of the SPE.
    pub mask: u64,
}

/// Fields 1 to 3: the spe id, the port id and the mask.
impl LaneValue for TransitionNotifier {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.spe);
        lanes.lane(2, 0, self.port);
        lanes.lane(3, 0, self.mask);
    }
}

/// The transition notifier of each logical SPE, by spe id.
///
/// An SPE with no notifier has no port and the mask 0, as the SPE
/// constructor leaves it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransitionNotifierState {
    notifiers: BTreeMap<u64, TransitionNotifier>,
}

impl TransitionNotifierState {
    /// Each notifier, by its spe id.
    pub fn notifiers(&self) -> &BTreeMap<u64, TransitionNotifier> {
        &self.notifiers
    }

    /// Keeps `notifier` in place of the notifier of its SPE.
    pub(crate) fn set(&mut self, notifier: TransitionNotifier) {
        self.notifiers.insert(notifier.spe, notifier);
    }

    /// Removes the notifier of the SPE `spe`.
    pub(crate) fn clear(&mut self, spe: u64) {
        self.notifiers.remove(&spe);
    }

    /// The sync-state terms of the notifiers.
    ///
    /// Each notifier is one object of the lane source
    /// `LOGICAL_SPE_NOTIFIER_SOURCE`, keyed by its index in spe id order.
    /// A state with no notifier adds no term.
    pub fn sync_term(&self) -> u128 {
        (0u64..)
            .zip(self.notifiers.values())
            .fold(0u128, |sum, (index, notifier)| {
                sum.wrapping_add(lanes::value_term(
                    LOGICAL_SPE_NOTIFIER_SOURCE,
                    index,
                    notifier,
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

/// Hypercall 64: the transition notifier of the SPE r3 gets the mask r4
/// and the event receive port r5, with r4 to r10 cleared.
///
/// The answers:
/// - r5 = 0: success, and the SPE has no notifier. lv1 does not read r4.
/// - r4 has a bit that a type 1 SPE does not accept:
///   [`SET_SPE_TRANSITION_NOTIFIER_MASK_R3`], and the notifier does not
///   change.
/// - otherwise: success, and the SPE keeps the notifier.
///
/// No call that CellGov answers changes the doubleword at +184 of an SPE.
/// So each SPE passes the check of lv1's call function.
///
/// The partition's object table holds objects that the hypervisor does
/// not keep. So lv1 can find an SPE or a port that the hypervisor does
/// not hold, and the call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 names no SPE that the hypervisor holds;
/// - r5 is not 0 and names no event receive port that the hypervisor
///   holds;
/// - r5 is not 0 and the SPE is not type 1, whose accepted mask no read
///   gives.
pub(crate) fn set_notifier(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spes: &LogicalSpeState,
    ports: &EventReceivePortState,
) -> HypercallDisposition {
    if firmware.is_none_or(|fw| spe_transition_notifier_handler(fw).is_none()) {
        return refuse(
            request,
            "an SPE transition notifier call at a firmware whose lv1 handler is not read",
        );
    }
    let Some(spe) = spes.spe(request.r(3)) else {
        return refuse(
            request,
            "an SPE transition notifier call on an spe id that the hypervisor did not construct",
        );
    };
    let port = request.r(5);
    if port == 0 {
        let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
        answer.push_change(SpeChange::NotifierCleared(spe.id));
        return HypercallDisposition::Answered(answer);
    }
    if ports.port(port).is_none() {
        return refuse(
            request,
            "an SPE transition notifier call on a port id that names no event receive port the \
             hypervisor constructed",
        );
    }
    if spe.spe_type != LOGICAL_SPE_TYPE_1 {
        return refuse(
            request,
            "an SPE transition notifier call on an SPE whose type's accepted mask no read gives",
        );
    }
    let mask = request.r(4);
    if mask & !LOGICAL_SPE_TYPE_1_TRANSITION_MASK != 0 {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            SET_SPE_TRANSITION_NOTIFIER_MASK_R3,
            [],
        ));
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(SpeChange::NotifierSet(TransitionNotifier {
        spe: spe.id,
        port,
        mask,
    }));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/transition_notifier_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/transition_notifier_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/transition_notifier_capture_tests.rs"]
mod capture_tests;
