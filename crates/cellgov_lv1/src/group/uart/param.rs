//! Hypercall 165, `get_virtual_uart_param`: the call reads one parameter
//! of a port of the partition's virtual UART.
//!
//! The answer carries no change. A console capture gives lv1's answers to
//! the GameOS kernel's calls for the buffer sizes of its ports. lv1's own
//! code can change those sizes while the partition runs, so CellGov gives
//! them only at those calls; see [`cellgov_ps3_abi::lv1::uart`]. The
//! interrupt status of a service port comes from the group's state; see
//! [`super::status`].

use cellgov_ps3_abi::lv1::call::GET_VIRTUAL_UART_PARAM;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::uart::{
    game_os_virtual_uart_port, is_game_os_virtual_uart_param_call, virtual_uart_param_handler,
    GET_VIRTUAL_UART_PARAM_NO_PARAM_R3, GET_VIRTUAL_UART_PARAM_NO_PORT_R3,
    VIRTUAL_UART_PARAM_INTERRUPT_STATUS, VIRTUAL_UART_PARAM_MAX, VIRTUAL_UART_PARAM_RX_BUFFER_SIZE,
    VIRTUAL_UART_PARAM_TX_BUFFER_SIZE, VIRTUAL_UART_PORT_MAX,
};

use super::status;
use super::UartState;
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: GET_VIRTUAL_UART_PARAM,
    name: "get_virtual_uart_param",
    cost: None,
    handler: |request, input| get_param(request, input.firmware, input.groups.uart()),
}];

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 165: the value of parameter r4 of port r3, in r4.
///
/// The answer clears r5 to r10. A call that fails one of lv1's checks
/// gets lv1's own r3, with r4 to r10 cleared. The checks, in lv1's order:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`]:
///   [`GET_VIRTUAL_UART_PARAM_NO_PORT_R3`];
/// - r4 is above [`VIRTUAL_UART_PARAM_MAX`]:
///   [`GET_VIRTUAL_UART_PARAM_NO_PARAM_R3`].
///
/// lv1 checks that it holds the port between the two checks. CellGov
/// knows only the ports in the capture, so it makes the second check only
/// for those ports.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the port r3 at `firmware`;
/// - r4 is a parameter other than the two buffer sizes and the interrupt
///   status. No capture gives lv1's value of another parameter. lv1 reads
///   the triggers and the mask from the port's endpoint, but its own code
///   can put a new endpoint, whose values are 0, in the port's slot at a
///   time that no image read shows;
/// - the call is not the GameOS kernel's call for that buffer size, at
///   its `sc` address, because no source gives the size at another time;
/// - [`status::port_status`] refuses the interrupt status.
pub(crate) fn get_param(
    request: &HypercallRequest,
    firmware: Option<&str>,
    uart: &UartState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| virtual_uart_param_handler(fw).is_some()) else {
        return refuse(
            request,
            "a virtual UART parameter call at a firmware whose lv1 handler is not read",
        );
    };
    let (port, param) = (request.r(3), request.r(4));
    if port > VIRTUAL_UART_PORT_MAX {
        return lv1_answer(GET_VIRTUAL_UART_PARAM_NO_PORT_R3);
    }
    let Some(row) = game_os_virtual_uart_port(firmware, port) else {
        return refuse(
            request,
            "a virtual UART port that no source shows lv1 holds or lacks",
        );
    };
    if param > VIRTUAL_UART_PARAM_MAX {
        return lv1_answer(GET_VIRTUAL_UART_PARAM_NO_PARAM_R3);
    }
    let value = match param {
        VIRTUAL_UART_PARAM_RX_BUFFER_SIZE => row.rx_buffer_size,
        VIRTUAL_UART_PARAM_TX_BUFFER_SIZE => row.tx_buffer_size,
        VIRTUAL_UART_PARAM_INTERRUPT_STATUS => {
            return match status::port_status(firmware, uart, port) {
                Ok(status) => success(status),
                Err(e) => e.refusal(request),
            };
        }
        _ => {
            return refuse(
                request,
                "a virtual UART parameter other than a buffer size or the interrupt status",
            )
        }
    };
    if !is_game_os_virtual_uart_param_call(firmware, request.pc, param) {
        return refuse(
            request,
            "a virtual UART buffer size call other than the GameOS kernel's",
        );
    }
    success(value)
}

/// lv1's r3 = 0, with `value` in r4 and r5 to r10 cleared.
fn success(value: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        zero_extended(LV1_SUCCESS),
        [value],
    ))
}

#[cfg(test)]
#[path = "tests/param_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/param_status_tests.rs"]
mod status_tests;
