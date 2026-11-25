//! Hypercall 138, `logical_spu_set_run_timeout`, and the run time limit
//! that the hypervisor keeps for each logical SPE.
//!
//! lv1 keeps the limit at +584 of the SPE object, and the SPE's start
//! step arms a timer from it; see [`cellgov_ps3_abi::lv1::spe`]. So the
//! hypervisor keeps the limit of each SPE, and hashes it.

use std::collections::BTreeMap;

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::LOGICAL_SPU_SET_RUN_TIMEOUT;
use cellgov_ps3_abi::lv1::spe::{spe_run_timeout_handler, LOGICAL_SPE_TYPE_1};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::construct::LogicalSpeState;
use super::{SpeChange, LOGICAL_SPE_RUN_TIMEOUT_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: LOGICAL_SPU_SET_RUN_TIMEOUT,
    name: "logical_spu_set_run_timeout",
    cost: None,
    handler: |request, input| set_run_timeout(request, input.firmware, input.groups.spe().spes()),
}];

/// The run time limit of one logical SPE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunTimeout {
    /// The spe id.
    pub spe: u64,
    /// The limit in timebase ticks, which lv1 keeps at +584 of the SPE.
    pub limit: u64,
}

/// Fields 1 and 2: the spe id and the limit.
impl LaneValue for RunTimeout {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.spe);
        lanes.lane(2, 0, self.limit);
    }
}

/// The run time limit of each logical SPE, by spe id.
///
/// An SPE with no entry has the limit 0, as the type 1 worker leaves it.
/// A limit of 0 removes the entry, so that the state of an SPE with the
/// limit 0 is one state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunTimeoutState {
    timeouts: BTreeMap<u64, RunTimeout>,
}

impl RunTimeoutState {
    /// Each limit that is not 0, by its spe id.
    pub fn timeouts(&self) -> &BTreeMap<u64, RunTimeout> {
        &self.timeouts
    }

    /// Keeps `timeout` in place of the limit of its SPE.
    pub(crate) fn set(&mut self, timeout: RunTimeout) {
        if timeout.limit == 0 {
            self.timeouts.remove(&timeout.spe);
        } else {
            self.timeouts.insert(timeout.spe, timeout);
        }
    }

    /// The sync-state terms of the limits.
    ///
    /// Each limit is one object of the lane source
    /// `LOGICAL_SPE_RUN_TIMEOUT_SOURCE`, keyed by its index in spe id
    /// order. A state with no limit adds no term.
    pub fn sync_term(&self) -> u128 {
        (0u64..)
            .zip(self.timeouts.values())
            .fold(0u128, |sum, (index, timeout)| {
                sum.wrapping_add(lanes::value_term(
                    LOGICAL_SPE_RUN_TIMEOUT_SOURCE,
                    index,
                    timeout,
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

/// Hypercall 138: the SPE r3 keeps the run time limit r4, with r4 to r10
/// cleared.
///
/// The hypervisor holds only SPEs that its `construct_logical_spe`
/// answer made. That answer passed the partition check and the flag
/// check that come first in lv1's worker. No call that CellGov answers
/// changes the doubleword at +184 of an SPE. So a type 1 SPE passes each
/// check of lv1's worker, call function and method, and the call
/// succeeds.
///
/// The partition's object table holds objects that the hypervisor does
/// not keep. So the call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 names no SPE that the hypervisor holds;
/// - the SPE is not type 1, whose doubleword at +192 no read gives.
pub(crate) fn set_run_timeout(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spes: &LogicalSpeState,
) -> HypercallDisposition {
    if firmware.is_none_or(|fw| spe_run_timeout_handler(fw).is_none()) {
        return refuse(
            request,
            "an SPE run timeout call at a firmware whose lv1 handler is not read",
        );
    }
    let Some(spe) = spes.spe(request.r(3)) else {
        return refuse(
            request,
            "an SPE run timeout call on an spe id that the hypervisor did not construct",
        );
    };
    if spe.spe_type != LOGICAL_SPE_TYPE_1 {
        return refuse(
            request,
            "an SPE run timeout call on an SPE whose type's +192 doubleword no read gives",
        );
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(SpeChange::RunTimeoutSet(RunTimeout {
        spe: spe.id,
        limit: request.r(4),
    }));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/run_timeout_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/run_timeout_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/run_timeout_boot_tests.rs"]
mod boot_tests;
