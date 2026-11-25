//! The interrupt status that lv1 computes for a port after each call of
//! `set_virtual_uart_param`, each `read_virtual_uart` or
//! `write_virtual_uart` that moves a byte, and each reply of a service,
//! and the mask bit that raises the port's interrupt.
//!
//! lv1 raises the port's interrupt when the status and the mask share a
//! bit; see [`cellgov_ps3_abi::lv1::uart`]. CellGov delivers no such
//! interrupt. So the hypervisor answers a call that leaves a mask other
//! than 0 only when these conditions are true:
//! - CellGov holds each field that the status reads, which is true only
//!   for a service port;
//! - the status and the mask share no bit.

use cellgov_ps3_abi::lv1::uart::{
    game_os_service_port, game_os_virtual_uart_port, virtual_uart_status, VirtualUartTraffic,
    VIRTUAL_UART_STATUS_RX, VIRTUAL_UART_STATUS_TX,
};

use super::VirtualUartPortParams;

/// The refusal of a service's reply that raises the receive interrupt.
pub(super) const REPLY_RAISE_REFUSAL: &str =
    "the delivery of the receive interrupt that lv1 raises on a virtual UART port when a \
     service's reply fills the port's receive buffer under the receive bit 2 of the port's mask, \
     to the thread that the port's irq plug names";

/// The refusal for the port's values `params` when lv1 can raise the
/// port's interrupt, with `received` bytes in the port's receive buffer.
///
/// The value is `None` in these cases:
/// - the mask is 0;
/// - the port is a service port, and its status and mask share no bit.
///
/// The refusal names one mask bit:
/// - for a service port, the lowest mask bit that raises the interrupt;
/// - for a port whose traffic CellGov does not hold, the lowest mask bit.
pub(super) fn mask_refusal(
    firmware: &str,
    params: &VirtualUartPortParams,
    received: u64,
) -> Option<&'static str> {
    match raised(firmware, params, received)? {
        Raised::ServicePort(bits) => Some(raise(lowest_bit(bits))),
        Raised::UnknownTraffic(mask) => Some(unknown_traffic(lowest_bit(mask))),
    }
}

/// The refusal for the port's values `params` when a service's reply
/// leaves `received` bytes in the port's receive buffer, and lv1 can
/// raise the port's interrupt.
///
/// lv1's endpoint write function calls the port's listeners after it
/// copies bytes to the port's receive buffer
/// [OFW-4.93 lv1 a:0x2F0324-0x2F03AC]. The refusal for the receive bit is
/// [`REPLY_RAISE_REFUSAL`]. The refusal for another bit is the refusal of
/// [`mask_refusal`].
pub(super) fn reply_refusal(
    firmware: &str,
    params: &VirtualUartPortParams,
    received: u64,
) -> Option<&'static str> {
    match raised(firmware, params, received)? {
        Raised::ServicePort(bits) if bits & VIRTUAL_UART_STATUS_RX != 0 => {
            Some(REPLY_RAISE_REFUSAL)
        }
        _ => mask_refusal(firmware, params, received),
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
/// bytes, so a mask with the transmit bit gets a refusal: lv1 can raise the
/// interrupt. A peer with more bytes changes only the transmit bit.
fn service_port_traffic(firmware: &str, port: u64, received: u64) -> Option<VirtualUartTraffic> {
    game_os_service_port(firmware, port)?;
    let row = game_os_virtual_uart_port(firmware, port)?;
    Some(VirtualUartTraffic {
        buffer_size: row.rx_buffer_size,
        buffer_count: received,
        peer_count: Some(0),
    })
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
