//! The interrupt status that lv1 computes for a port after each call of
//! `set_virtual_uart_param`, each `read_virtual_uart` or
//! `write_virtual_uart` that moves a byte, and each reply of a service,
//! and the mask bit that raises the port's interrupt. `get_virtual_uart_param`
//! gives the same status for parameter 7.
//!
//! lv1 sets the port's bit in the virtual UART's shadow while the status
//! and the mask share a bit, and clears it otherwise; see
//! [`cellgov_ps3_abi::lv1::uart`] and [`super::shadow`]. CellGov decides
//! the step only when it holds each field that the status reads:
//! - for a service port, the receive bit follows the port's own receive
//!   buffer, which the hypervisor holds;
//! - the transmit bit follows the bytes in the peer's buffer, which the
//!   service reads at a time that no image read gives.

use cellgov_ps3_abi::lv1::uart::{
    game_os_service_port, game_os_virtual_uart_port, virtual_uart_status, VirtualUartTraffic,
    VIRTUAL_UART_STATUS_RX, VIRTUAL_UART_STATUS_TX,
};

use super::{UartState, VirtualUartPortParams};
use crate::answer::UnmodelledError;

/// What lv1's status function does to a port's bit in the virtual UART's
/// shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShadowStep {
    /// The bit stays as it is.
    Keep,
    /// The function sets the bit [OFW-4.93 lv1 a:0x2F225C-0x2F2278].
    Raise,
    /// The function clears the bit [OFW-4.93 lv1 a:0x2F2308-0x2F2334].
    Clear,
}

/// lv1's step for the port's values `params`, with `received` bytes in
/// the port's receive buffer, when `raised_bit` is the port's shadow bit.
///
/// The status function changes nothing for a bit already in the state
/// that the status gives [OFW-4.93 lv1 a:0x2F2250-0x2F2258]
/// [OFW-4.93 lv1 a:0x2F2308-0x2F2310].
///
/// # Errors
///
/// Returns the refusal text, which names one mask bit, in these cases:
/// - for a service port, the status and the mask share the transmit bit
///   and not the receive bit, so the step depends on the peer's bytes;
/// - for a port whose traffic CellGov does not hold, the mask is not 0.
pub(super) fn shadow_step(
    firmware: &str,
    params: &VirtualUartPortParams,
    received: u64,
    raised_bit: bool,
) -> Result<ShadowStep, UnmodelledError> {
    let step = |shared: bool| match (shared, raised_bit) {
        (true, false) => ShadowStep::Raise,
        (false, true) => ShadowStep::Clear,
        _ => ShadowStep::Keep,
    };
    match raised(firmware, params, received) {
        None => Ok(step(false)),
        Some(Raised::ServicePort(bits)) if bits & VIRTUAL_UART_STATUS_RX != 0 => Ok(step(true)),
        Some(Raised::ServicePort(bits)) => Err(UnmodelledError::new(raise(lowest_bit(bits)))),
        Some(Raised::UnknownTraffic(mask)) => {
            Err(UnmodelledError::new(unknown_traffic(lowest_bit(mask))))
        }
    }
}

/// The mask bits with which lv1 can raise a port's interrupt.
enum Raised {
    /// The bits that the status of a service port and its mask share.
    ServicePort(u64),
    /// The mask of a port whose traffic CellGov does not hold. Each of
    /// its bits can raise the interrupt.
    UnknownTraffic(u64),
}

/// The mask bits with which lv1 can raise the port's interrupt, or `None`
/// when it raises none.
fn raised(firmware: &str, params: &VirtualUartPortParams, received: u64) -> Option<Raised> {
    let mask = params.interrupt_mask;
    if mask == 0 {
        return None;
    }
    let Some(traffic) = service_port_traffic(firmware, params.port, received) else {
        return Some(Raised::UnknownTraffic(mask));
    };
    let bits = virtual_uart_status(traffic, params.tx_trigger, params.rx_trigger) & mask;
    (bits != 0).then_some(Raised::ServicePort(bits))
}

/// The traffic of the service port `port` at `firmware`, with `received`
/// bytes in its receive buffer, or `None` when `port` is not a service
/// port.
///
/// The fields come from these facts:
/// - The buffer size is the size that the capture reads at the kernel's
///   open; see [`game_os_virtual_uart_port`].
/// - The receive buffer holds the bytes of the service's replies that the
///   partition did not read, and the byte at +128 of the endpoint is 0.
///   `read_virtual_uart` reads those bytes from the port on the same
///   facts.
/// - The port has a peer with a reference. The capture gives the port a
///   transmit buffer size other than 0; see
///   [`cellgov_ps3_abi::lv1::uart::VIRTUAL_UART_PARAM_TX_BUFFER_SIZE`].
/// - lv1's write function puts the bytes that the port sends in the
///   peer's receive buffer [OFW-4.93 lv1 a:0x2F04E4-0x2F0530]. The
///   service reads them at a time that no image read gives. It reads each
///   byte of a request before it replies. So the peer's receive buffer
///   holds 0 bytes, or at most the bytes of a request that is not
///   complete.
///
/// The traffic gives the peer 0 bytes. lv1 sets the transmit bit for 0
/// bytes, so a mask with the transmit bit and no shared receive bit gets a
/// refusal; see [`shadow_step`]. A peer with more bytes changes only the
/// transmit bit.
fn service_port_traffic(firmware: &str, port: u64, received: u64) -> Option<VirtualUartTraffic> {
    game_os_service_port(firmware, port)?;
    let row = game_os_virtual_uart_port(firmware, port)?;
    Some(VirtualUartTraffic {
        buffer_size: row.rx_buffer_size,
        buffer_count: received,
        peer_count: Some(0),
    })
}

/// The interrupt status that `get_virtual_uart_param` gives for port
/// `port` in `uart`.
///
/// The status reads the traffic of [`service_port_traffic`], with one
/// change: the peer holds the bytes of the request that is not complete
/// on the port, or a lower count. The service reads those bytes at a time
/// that no image read gives. The transmit bit is set when the peer's count
/// is not above the transmit trigger, so the status is known when the two
/// ends of that range give the same status.
///
/// # Errors
///
/// Returns the case that the model does not cover in these cases:
/// - `port` is not a service port, so CellGov does not hold its traffic;
/// - the two ends of the peer's range give two statuses.
pub(super) fn port_status(
    firmware: &str,
    uart: &UartState,
    port: u64,
) -> Result<u64, UnmodelledError> {
    let params = uart.port_params(port);
    let received = uart.received(port).len() as u64;
    let Some(traffic) = service_port_traffic(firmware, port, received) else {
        return Err(UnmodelledError::new(
            "the interrupt status of a virtual UART port whose peer and traffic no source gives",
        ));
    };
    let unread = VirtualUartTraffic {
        peer_count: Some(uart.pending_request(port).sent),
        ..traffic
    };
    let status = virtual_uart_status(traffic, params.tx_trigger, params.rx_trigger);
    if virtual_uart_status(unread, params.tx_trigger, params.rx_trigger) != status {
        return Err(UnmodelledError::new(
            "the interrupt status of a virtual UART port whose peer can hold more bytes than the \
             transmit trigger",
        ));
    }
    Ok(status)
}

/// The lowest set bit of `bits`, or 0 when `bits` is 0.
const fn lowest_bit(bits: u64) -> u64 {
    bits & bits.wrapping_neg()
}

/// The refusal for the mask bit `bit`, with which lv1 raises the port's
/// interrupt.
fn raise(bit: u64) -> &'static str {
    match bit {
        VIRTUAL_UART_STATUS_TX => {
            "the transmit bit 1 of a virtual UART interrupt mask on a port whose peer holds no \
             more bytes than the transmit trigger, with which lv1 raises the port's interrupt"
        }
        VIRTUAL_UART_STATUS_RX => {
            "the receive bit 2 of a virtual UART interrupt mask on a port whose receive buffer \
             has no more free bytes than the receive trigger, with which lv1 raises the port's \
             interrupt"
        }
        _ => {
            "the disconnect bit 4 of a virtual UART interrupt mask on a port with no peer, with \
             which lv1 raises the port's interrupt"
        }
    }
}

/// The refusal for the mask bit `bit` on a port whose traffic CellGov
/// does not hold.
fn unknown_traffic(bit: u64) -> &'static str {
    match bit {
        VIRTUAL_UART_STATUS_TX => {
            "the transmit bit 1 of a virtual UART interrupt mask on a port whose peer and \
             traffic no source gives"
        }
        VIRTUAL_UART_STATUS_RX => {
            "the receive bit 2 of a virtual UART interrupt mask on a port whose peer and traffic \
             no source gives"
        }
        _ => {
            "the disconnect bit 4 of a virtual UART interrupt mask on a port whose peer and \
             traffic no source gives"
        }
    }
}

#[cfg(test)]
#[path = "tests/status_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/status_received_tests.rs"]
mod received_tests;
