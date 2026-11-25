//! Hypercall 91, `read_repository_node`, answered from the compiled-in
//! node table.

use cellgov_ps3_abi::lv1::repository::{lookup, RepositoryKey};

use super::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use super::request::HypercallRequest;

/// Answers the node that r3 to r7 name, with its two values in r4 and r5.
///
/// The key compares as raw words, the partition included. A node no
/// source gives values for is a refusal that names the key. The read
/// never answers `LV1_NO_ENTRY` on its own.
pub(super) fn read_node(request: &HypercallRequest) -> HypercallDisposition {
    let key = RepositoryKey {
        lpar: request.r(3),
        n1: request.r(4),
        n2: request.r(5),
        n3: request.r(6),
        n4: request.r(7),
    };
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

#[cfg(test)]
#[path = "tests/repository_tests.rs"]
mod tests;
