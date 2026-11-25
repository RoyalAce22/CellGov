//! The interrupt status that lv1 computes for a port after each call of
//! `set_virtual_uart_param` and each `write_virtual_uart` that writes a
//! byte, and the mask bit that raises the port's interrupt.
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

/// The refusal for the port's values `params` when lv1 can raise the port's interrupt.
///
/// The value is `None` in these cases:
/// - the mask is 0;
/// - the port is a service port, and its status and mask share no bit.
///
/// The refusal names one mask bit:
/// - for a service port, the lowest mask bit that raises the interrupt;
/// - for a port whose traffic CellGov does not hold, the lowest mask bit.
pub(super) fn mask_refusal(firmware: &str, params: &VirtualUartPortParams) -> Option<&'static str> {
    let mask = params.interrupt_mask;
    if mask == 0 {
        return None;
    }
    let Some(traffic) = service_port_traffic(firmware, params.port) else {
        return Some(unknown_traffic(lowest_bit(mask)));
    };
    let raised = virtual_uart_status(traffic, params.tx_trigger, params.rx_trigger) & mask;
    (raised != 0).then(|| raise(lowest_bit(raised)))
}

/// The traffic of the service port `port` at `firmware`, or `None` when
/// `port` is not a service port.
///
/// The fields come from these facts:
/// - The buffer size is the size that the capture reads at the kernel's
///   open; see [`game_os_virtual_uart_port`].
/// - The receive buffer holds no byte, and the byte at +128 of the
///   endpoint is 0. `read_virtual_uart` reads 0 bytes from the port on
///   the same facts.
/// - The port has a peer with a reference. The capture gives the port a
///   transmit buffer size other than 0; see
///   [`cellgov_ps3_abi::lv1::uart::VIRTUAL_UART_PARAM_TX_BUFFER_SIZE`].
/// - lv1's write function puts the bytes that the port sends in the
///   peer's receive buffer [OFW-4.93 lv1 a:0x2F04E4-0x2F0530]. CellGov
///   answers `write_virtual_uart` only for bytes of a request that is not
///   complete. The service reads them at a time that no image read gives.
///   So the peer's receive buffer holds 0 bytes, or at most the bytes of
///   that request.
///
/// The traffic gives the peer 0 bytes. lv1 sets the transmit bit for 0
/// bytes, so a mask with the transmit bit gets a refusal: lv1 can raise the
/// interrupt. A peer with more bytes changes only the transmit bit.
fn service_port_traffic(firmware: &str, port: u64) -> Option<VirtualUartTraffic> {
    game_os_service_port(firmware, port)?;
    let row = game_os_virtual_uart_port(firmware, port)?;
    Some(VirtualUartTraffic {
        buffer_size: row.rx_buffer_size,
        buffer_count: 0,
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
