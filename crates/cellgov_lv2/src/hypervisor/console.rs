//! The logical console: the guest TTY sink that hypercalls 105, 107 and
//! 109 reach.
//!
//! CellGov's console holds no buffer. A send gives its bytes to the sink
//! whole, so a flush has nothing left to move. The console takes bytes
//! for any console id.

use cellgov_ps3_abi::lv1::call::{LOGICAL_CONSOLE_SEND, LOGICAL_CONSOLE_SEND_MAX};

use super::answer::{ConsoleWrite, HypercallAnswer, HypercallDisposition, HypercallRefusal};
use super::request::HypercallRequest;

/// Hypercall 105: the console exists, so the construct succeeds.
pub(super) fn construct() -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::success(Vec::new()))
}

/// Hypercall 107: the sink takes the first r4 bytes of r5 to r8, and the
/// answer puts that count in r4.
///
/// A count above [`LOGICAL_CONSOLE_SEND_MAX`] is a refusal: no source
/// says what lv1 does with it.
pub(super) fn send(request: &HypercallRequest) -> HypercallDisposition {
    let len = request.r(4);
    let Ok(n) = u8::try_from(len) else {
        return too_long(request);
    };
    if usize::from(n) > LOGICAL_CONSOLE_SEND_MAX {
        return too_long(request);
    }
    let mut data = [0u8; LOGICAL_CONSOLE_SEND_MAX];
    for (chunk, reg) in data.chunks_exact_mut(8).zip(5..=8) {
        chunk.copy_from_slice(&request.r(reg).to_be_bytes());
    }
    let mut answer = HypercallAnswer::success(vec![len]);
    answer.console = Some(ConsoleWrite {
        console: request.r(3),
        data,
        len: n,
    });
    HypercallDisposition::Answered(answer)
}

fn too_long(request: &HypercallRequest) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: LOGICAL_CONSOLE_SEND,
        unit: request.unit,
        pc: request.pc,
        what: "a console send longer than the 32 bytes r5 to r8 hold",
    })
}

/// Hypercall 109: the console holds no bytes, so the flush succeeds.
pub(super) fn flush() -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::success(Vec::new()))
}

#[cfg(test)]
#[path = "tests/console_tests.rs"]
mod tests;
