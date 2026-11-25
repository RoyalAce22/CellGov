use super::*;
use crate::answer::UnmodelledError;
use crate::answer::{HypercallAnswer, HypercallDisposition};
use crate::group::interrupt::raise::RaiseState;
use crate::group::interrupt::InterruptGroupState;
use crate::group::uart::receive::ReceivedBytes;
use crate::group::uart::shadow::{StatusInput, VirtualUartShadowWrite};
use crate::group::uart::{UartChange, UartState};
use crate::request::HypercallRequest;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::SET_VIRTUAL_UART_PARAM;
use cellgov_ps3_abi::lv1::uart::GAME_OS_VIRTUAL_UART_PORTS;
use cellgov_time::GuestTicks;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

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
    let step = |rx, received| shadow_step("4.93", &params(rx, 2), received, false);
    assert_eq!(step(rx, 0), Ok(ShadowStep::Keep));
    assert_eq!(step(rx, 1), Ok(ShadowStep::Raise));
    let rx = size() - 0x48;
    assert_eq!(step(rx, 0x47), Ok(ShadowStep::Keep));
    assert_eq!(step(rx, 0x48), Ok(ShadowStep::Raise));
}

/// A set bit stays while the status shares it, and clears once the
/// status no longer does [OFW-4.93 lv1 a:0x2F2250-0x2F2258]
/// [OFW-4.93 lv1 a:0x2F2308-0x2F2334].
#[test]
fn a_raised_bit_stays_while_shared_and_clears_once_the_bytes_are_read() {
    let rx = size() - 1;
    let step = |received| shadow_step("4.93", &params(rx, 2), received, true);
    assert_eq!(step(0x48), Ok(ShadowStep::Keep));
    assert_eq!(step(0), Ok(ShadowStep::Clear));
}

/// The receive bit decides the step when the mask holds the transmit bit
/// too: any shared bit sets the port's one shadow bit
/// [OFW-4.93 lv1 a:0x2F221C-0x2F224C].
#[test]
fn a_reply_under_the_receive_and_transmit_bits_raises_the_shadow_bit() {
    assert_eq!(
        shadow_step("4.93", &params(size() - 1, 3), 0x48, false),
        Ok(ShadowStep::Raise)
    );
}

/// The peer holds no byte, which is not above the transmit trigger 0.
#[test]
fn a_reply_under_the_transmit_bit_alone_gets_the_transmit_refusal() {
    assert_eq!(
        shadow_step("4.93", &params(size() - 1, 1), 0x48, false),
        Err(UnmodelledError::new(TX_RAISE))
    );
}

/// lv1 computes the status after the mask call
/// [OFW-4.93 lv1 a:0x2F3FB0-0x2F4008].
#[test]
fn a_mask_call_on_a_port_whose_buffer_holds_bytes_raises_its_shadow_bit() {
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
    let set = |uart: &UartState| {
        let interrupt = InterruptGroupState::default();
        let storage = GuestMemory::new(0);
        let input = StatusInput {
            uart,
            raise: RaiseState {
                bitmaps: interrupt.bitmaps(),
                ports: interrupt.ports(),
                paused: interrupt.paused(),
                storage: &storage,
            },
        };
        crate::group::uart::set_param::set_param(&request(2), Some("4.93"), &input)
    };
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(params(size() - 1, 0)));
    let mut empty = HypercallAnswer::lv1_status(0, [0; 7]);
    empty.push_change(UartChange::PortParamsSet(params(size() - 1, 2)));
    assert_eq!(set(&uart), HypercallDisposition::Answered(empty));
    uart.apply(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: PORT,
        bytes: vec![0; 0x48],
    }));
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(UartChange::PortParamsSet(params(size() - 1, 2)));
    expected.push_change(UartChange::ShadowSet(VirtualUartShadowWrite {
        index: 0,
        value: 1 << 53,
    }));
    assert_eq!(set(&uart), HypercallDisposition::Answered(expected));
}
