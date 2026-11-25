//! Hypercall 91, `read_repository_node`, answered from the compiled-in
//! node table.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{lookup, RepositoryKey};

use super::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use super::request::HypercallRequest;

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

/// Answers the node that r3 to r7 name, with its two values in r4 and r5.
///
/// The key compares as raw words, the partition included. A node no
/// source gives values for is a refusal that names the key. The read
/// never answers `LV1_NO_ENTRY` on its own.
pub(super) fn read_node(request: &HypercallRequest) -> HypercallDisposition {
    let key = key_of(request);
    match lookup(&key) {
        Some(node) => {
            HypercallDisposition::Answered(HypercallAnswer::success(vec![node.v1, node.v2]))
        }
        None => HypercallDisposition::Refused(HypercallRefusal::RepositoryNode {
            key,
            unit: request.unit,
            pc: request.pc,
        }),
    }
}

/// The record of the read `request` made, which `read_node` answered
/// with `disposition`.
pub(super) fn read_record(
    request: &HypercallRequest,
    disposition: &HypercallDisposition,
) -> RepositoryRead {
    let values = match disposition {
        HypercallDisposition::Answered(answer) => match answer.outputs.as_slice() {
            [v1, v2] => Some((*v1, *v2)),
            _ => None,
        },
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
