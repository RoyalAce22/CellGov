//! Hypercall 91, `read_repository_node`, answered from the compiled-in
//! node table.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE;
use cellgov_ps3_abi::lv1::repository::{lookup, ReadSite, RepositoryKey, SecondValue};

use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: READ_REPOSITORY_NODE,
    name: "read_repository_node",
    cost: None,
    handler: |request, input| read_node(request, input.firmware),
}];

/// One repository read the kernel made, in the order of the reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryRead {
    /// The partition and the raw key words of the read.
    pub key: RepositoryKey,
    /// The two values the read answered, or `None` when the hypervisor
    /// refused the read.
    pub values: Option<(u64, u64)>,
    /// The unit that made the call.
    pub unit: UnitId,
    /// The address of its `sc`.
    pub pc: u64,
}

/// The partition and the raw key words that r3 to r7 of `request` name.
fn key_of(request: &HypercallRequest) -> RepositoryKey {
    RepositoryKey {
        lpar: request.r(3),
        n1: request.r(4),
        n2: request.r(5),
        n3: request.r(6),
        n4: request.r(7),
    }
}

/// Answers the node that r3 to r7 name at the firmware version
/// `firmware`, with its two values in r4 and r5.
///
/// The key compares as raw words, the partition included. The read is a
/// refusal that names the key in these cases:
/// - no source gives the node's values for `firmware`;
/// - `firmware` is `None`;
/// - the source does not observe the second value, and the read site is
///   not one that discards it.
///
/// At a site that discards an unobserved second value, r5 gets 0. The
/// read never answers `LV1_NO_ENTRY` on its own.
pub(crate) fn read_node(
    request: &HypercallRequest,
    firmware: Option<&str>,
) -> HypercallDisposition {
    let key = key_of(request);
    let Some(node) = firmware.and_then(|firmware| lookup(&key, firmware)) else {
        return HypercallDisposition::Refused(HypercallRefusal::RepositoryNode {
            key,
            unit: request.unit,
            pc: request.pc,
        });
    };
    let site = ReadSite {
        sc: request.pc,
        lr: request.lr,
    };
    match node.v2 {
        SecondValue::Given(v2) => {
            HypercallDisposition::Answered(HypercallAnswer::success([node.v1, v2]))
        }
        SecondValue::Unobserved { discarded_by } if discarded_by.contains(&site) => {
            HypercallDisposition::Answered(HypercallAnswer::success([node.v1, 0]))
        }
        SecondValue::Unobserved { .. } => {
            HypercallDisposition::Refused(HypercallRefusal::RepositorySecondValueUnobserved {
                key,
                unit: request.unit,
                pc: request.pc,
                lr: request.lr,
            })
        }
    }
}

/// The record of the read `request` made, which `read_node` answered
/// with `disposition`.
pub(crate) fn read_record(
    request: &HypercallRequest,
    disposition: &HypercallDisposition,
) -> RepositoryRead {
    let values = match disposition {
        HypercallDisposition::Answered(answer) => answer.outputs().map(|[v1, v2, ..]| (v1, v2)),
        HypercallDisposition::Refused(_) => None,
    };
    RepositoryRead {
        key: key_of(request),
        values,
        unit: request.unit,
        pc: request.pc,
    }
}

#[cfg(test)]
#[path = "tests/repository_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/repository_firmware_tests.rs"]
mod firmware_tests;

#[cfg(test)]
#[path = "tests/repository_reader_tests.rs"]
mod reader_tests;

#[cfg(test)]
#[path = "tests/repository_debug_support_tests.rs"]
mod debug_support_tests;

#[cfg(test)]
#[path = "tests/console_repository_debug_support_tests.rs"]
mod console_tests;
