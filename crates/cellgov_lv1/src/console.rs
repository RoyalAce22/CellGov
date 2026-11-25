//! The logical console: the guest TTY sink that hypercalls 105 to 107
//! and 109 reach.
//!
//! CellGov's console holds no buffer. A send gives its bytes to the sink
//! whole, so a flush has nothing left to move. The console takes bytes
//! for any console id.
//!
//! The answers are the ones lv1 gives with its console gate off, as on
//! the reference console; see [`cellgov_ps3_abi::lv1::console`]. With the
//! gate off, lv1 keeps no console state, so no call here keeps state.
//! The sink is CellGov's own: lv1 drops the bytes of a send.

use cellgov_ps3_abi::lv1::call::{LOGICAL_CONSOLE_SEND, LOGICAL_CONSOLE_SEND_MAX};
use cellgov_ps3_abi::lv1::console::console_gate_off;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::answer::{ConsoleWrite, HypercallAnswer, HypercallDisposition, HypercallRefusal};
use super::request::HypercallRequest;
use super::vector;

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

/// Hypercall 106: with the gate off, lv1 puts 0 in r3 for any console id
/// and does nothing else.
///
/// r4 to r10 get 0: lv1 leaves data from its own stack there, and
/// CellGov does not model that data. The kernel reads r3 alone.
///
/// The call is a refusal at a firmware where no console capture shows the
/// gate off. Each such capture needs the lv1 worker read.
pub(super) fn destruct(request: &HypercallRequest, firmware: Option<&str>) -> HypercallDisposition {
    if !firmware.is_some_and(console_gate_off) {
        return HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: request.number,
            unit: request.unit,
            pc: request.pc,
            what: "a console destruct at a firmware whose console gate no capture gives",
        });
    }
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        zero_extended(LV1_SUCCESS),
        vector::cleared_outputs(),
    ))
}

#[cfg(test)]
#[path = "tests/console_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/console_destruct_tests.rs"]
mod destruct_tests;

#[cfg(test)]
#[path = "tests/console_lv2_console_object_tests.rs"]
mod console_object_tests;
