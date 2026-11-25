use super::*;
use crate::answer::{HypercallDisposition, HypercallRefusal};
use crate::group::uart::receive::ReceivedBytes;
use crate::group::uart::set_param::set_param;
use crate::group::uart::{UartChange, UartState};
use crate::request::HypercallRequest;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::SET_VIRTUAL_UART_PARAM;
use cellgov_ps3_abi::lv1::uart::GAME_OS_VIRTUAL_UART_PORTS;
use cellgov_time::GuestTicks;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

const RX_RAISE: &str = "the receive bit 2 of a virtual UART interrupt mask on a port whose \
                        receive buffer has no more free bytes than the receive trigger, with \
                        which lv1 raises the port's interrupt";

const TX_RAISE: &str = "the transmit bit 1 of a virtual UART interrupt mask on a port whose \
                        peer holds no more bytes than the transmit trigger, with which lv1 \
                        raises the port's interrupt";

/// The size of the receive buffer of [`PORT`] at the kernel's open.
fn size() -> u64 {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == "4.93" && row.port == PORT)
        .expect("a row for the port")
        .rx_buffer_size
}

fn params(rx: u64, mask: u64) -> VirtualUartPortParams {
    VirtualUartPortParams {
        port: PORT,
        tx_trigger: 0,
        rx_trigger: rx,
        interrupt_mask: mask,
    }
}

/// The status has the receive bit when the buffer size less the count is
/// not above the receive trigger [OFW-4.93 lv1 a:0x2F2208-0x2F2210].
#[test]
fn mask_2_raises_once_the_received_bytes_bring_the_free_bytes_to_the_receive_trigger() {
    let rx = size() - 1;
    assert_eq!(mask_refusal("4.93", &params(rx, 2), 0), None);
    assert_eq!(mask_refusal("4.93", &params(rx, 2), 1), Some(RX_RAISE));
    let rx = size() - 0x48;
    assert_eq!(mask_refusal("4.93", &params(rx, 2), 0x47), None);
    assert_eq!(mask_refusal("4.93", &params(rx, 2), 0x48), Some(RX_RAISE));
}

#[test]
fn a_reply_that_raises_the_receive_bit_gets_the_reply_refusal() {
    let rx = size() - 1;
    assert_eq!(reply_refusal("4.93", &params(rx, 2), 0), None);
    assert_eq!(
        reply_refusal("4.93", &params(rx, 2), 0x48),
        Some(REPLY_RAISE_REFUSAL)
    );
    assert_eq!(
        reply_refusal("4.93", &params(rx, 3), 0x48),
        Some(REPLY_RAISE_REFUSAL)
    );
}

/// The peer holds no byte, which is not above the transmit trigger 0.
#[test]
fn a_reply_under_the_transmit_bit_alone_gets_the_transmit_refusal() {
    assert_eq!(
        reply_refusal("4.93", &params(size() - 1, 1), 0x48),
        Some(TX_RAISE)
    );
}

/// lv1 computes the status after the mask call
/// [OFW-4.93 lv1 a:0x2F3FB0-0x2F4008].
#[test]
fn a_mask_call_on_a_port_whose_buffer_holds_bytes_counts_them() {
    let request = |mask| {
        let mut inputs = [0; 8];
        inputs[..3].copy_from_slice(&[PORT, 2, mask]);
        HypercallRequest {
            unit: UnitId::new(1),
            number: SET_VIRTUAL_UART_PARAM,
            inputs,
            pc: 0x8000_0000_0001_6ac8,
            lr: 0,
            thread: Some(1),
            problem_state: false,
            tick: GuestTicks::new(1),
        }
    };
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(params(size() - 1, 0)));
    assert!(matches!(
        set_param(&request(2), Some("4.93"), &uart),
        HypercallDisposition::Answered(_)
    ));
    uart.apply(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: PORT,
        bytes: vec![0; 0x48],
    }));
    let r = request(2);
    assert_eq!(
        set_param(&r, Some("4.93"), &uart),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: SET_VIRTUAL_UART_PARAM,
            unit: r.unit,
            pc: r.pc,
            what: RX_RAISE,
        })
    );
}
