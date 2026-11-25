//! Hypercall 164, `set_virtual_uart_param`: the call sets the transmit
//! trigger, the receive trigger or the interrupt mask of a port of the
//! partition's virtual UART.
//!
//! lv1 keeps the three values in the port's endpoint. After each call it
//! computes the port's interrupt status, and raises the port's interrupt
//! when the status and the mask share a bit; see
//! [`cellgov_ps3_abi::lv1::uart`]. CellGov answers a call that raises no
//! interrupt, and keeps and hashes the values of each port.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::SET_VIRTUAL_UART_PARAM;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::uart::{
    game_os_virtual_uart_port, virtual_uart_set_param_handler, SET_VIRTUAL_UART_PARAM_MASK_R3,
    SET_VIRTUAL_UART_PARAM_NO_PARAM_R3, SET_VIRTUAL_UART_PARAM_NO_PORT_R3,
    VIRTUAL_UART_INTERRUPT_MASK_MAX, VIRTUAL_UART_PARAM_INTERRUPT_MASK,
    VIRTUAL_UART_PARAM_RX_TRIGGER, VIRTUAL_UART_PARAM_TX_TRIGGER, VIRTUAL_UART_PORT_MAX,
};

use super::status;
use super::{UartChange, UartState};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: SET_VIRTUAL_UART_PARAM,
    name: "set_virtual_uart_param",
    cost: None,
    handler: |request, input| set_param(request, input.firmware, input.groups.uart()),
}];

/// The values that `set_virtual_uart_param` gave one port of the virtual
/// UART.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartPortParams {
    /// The port number.
    pub port: u64,
    /// The transmit trigger, which lv1 keeps at +136 of the endpoint.
    pub tx_trigger: u64,
    /// The receive trigger, which lv1 keeps at +144 of the endpoint.
    pub rx_trigger: u64,
    /// The interrupt mask, which lv1 keeps at +152 of the endpoint.
    pub interrupt_mask: u64,
}

impl VirtualUartPortParams {
    /// The values of port `port` before a call sets one. The endpoint
    /// constructor sets each value to 0.
    pub const fn initial(port: u64) -> Self {
        Self {
            port,
            tx_trigger: 0,
            rx_trigger: 0,
            interrupt_mask: 0,
        }
    }
}

/// Fields 1 to 4: the port number, the transmit trigger, the receive
/// trigger and the interrupt mask.
impl LaneValue for VirtualUartPortParams {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.port);
        lanes.lane(2, 0, self.tx_trigger);
        lanes.lane(3, 0, self.rx_trigger);
        lanes.lane(4, 0, self.interrupt_mask);
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

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 164: sets parameter r4 of port r3 to r5.
///
/// The answer returns 0 in r3 and clears r4 to r10. It carries the port's
/// new values, and the hypervisor keeps them. A call that fails one of
/// lv1's checks gets lv1's own r3, with r4 to r10 cleared. The checks, in
/// lv1's order:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`]:
///   [`SET_VIRTUAL_UART_PARAM_NO_PORT_R3`];
/// - r4 is not one of the three parameters:
///   [`SET_VIRTUAL_UART_PARAM_NO_PARAM_R3`];
/// - r4 is the mask, and r5 is above [`VIRTUAL_UART_INTERRUPT_MASK_MAX`]:
///   [`SET_VIRTUAL_UART_PARAM_MASK_R3`].
///
/// lv1 checks that it holds the port between the first two checks.
/// CellGov knows only the ports in the capture, so it makes the later
/// checks only for those ports.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the port r3 at `firmware`;
/// - the call leaves a mask other than 0, and lv1 raises the port's
///   interrupt, or CellGov does not hold the port's traffic. The refusal
///   names the mask bit; see [`status::mask_refusal`].
pub(crate) fn set_param(
    request: &HypercallRequest,
    firmware: Option<&str>,
    uart: &UartState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| virtual_uart_set_param_handler(fw).is_some()) else {
        return refuse(
            request,
            "a virtual UART parameter write at a firmware whose lv1 handler is not read",
        );
    };
    let (port, param, value) = (request.r(3), request.r(4), request.r(5));
    if port > VIRTUAL_UART_PORT_MAX {
        return lv1_answer(SET_VIRTUAL_UART_PARAM_NO_PORT_R3);
    }
    if game_os_virtual_uart_port(firmware, port).is_none() {
        return refuse(
            request,
            "a virtual UART port that no source shows lv1 holds or lacks",
        );
    }
    let mut params = uart.port_params(port);
    match param {
        VIRTUAL_UART_PARAM_TX_TRIGGER => params.tx_trigger = value,
        VIRTUAL_UART_PARAM_RX_TRIGGER => params.rx_trigger = value,
        VIRTUAL_UART_PARAM_INTERRUPT_MASK if value > VIRTUAL_UART_INTERRUPT_MASK_MAX => {
            return lv1_answer(SET_VIRTUAL_UART_PARAM_MASK_R3);
        }
        VIRTUAL_UART_PARAM_INTERRUPT_MASK => params.interrupt_mask = value,
        _ => return lv1_answer(SET_VIRTUAL_UART_PARAM_NO_PARAM_R3),
    }
    if let Some(what) = status::mask_refusal(firmware, &params) {
        return refuse(request, what);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(UartChange::PortParamsSet(params));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/set_param_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/set_param_hash_tests.rs"]
mod hash_tests;
