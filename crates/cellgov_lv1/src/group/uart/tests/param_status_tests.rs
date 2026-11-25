use super::*;
use crate::group::uart::{PendingRequest, ReceivedBytes, UartChange, VirtualUartPortParams};
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::SET_VIRTUAL_UART_PARAM;
use cellgov_ps3_abi::lv1::uart::{
    GAME_OS_VIRTUAL_UART_PORTS, VIRTUAL_UART_PARAM_RX_TRIGGER, VIRTUAL_UART_STATUS_CONNECTED,
    VIRTUAL_UART_STATUS_RX, VIRTUAL_UART_STATUS_TX,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's interrupt status call
/// [OFW-4.93 lv2_kernel a:0x80000000000174CC-0x80000000000174E4].
const KERNEL_STATUS_SC: u64 = 0x8000_0000_0001_74E4;

/// The system manager's port [OFW-4.93 lv1 a:0x44CF04-0x44CF24].
const SYSTEM_MANAGER_PORT: u64 = 2;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E44-0x5B7E80].
const DISPATCHER_PORT: u64 = 10;

/// The status of a port with a peer whose count is not above the
/// transmit trigger [OFW-4.93 lv1 a:0x2F0D48-0x2F0D54]
/// [OFW-4.93 lv1 a:0x2F0D6C-0x2F0D7C].
const CONNECTED_TX: u64 = VIRTUAL_UART_STATUS_CONNECTED | VIRTUAL_UART_STATUS_TX;

const UNREAD_REFUSAL: &str = "the interrupt status of a virtual UART port whose peer can hold \
                              more bytes than the transmit trigger";

/// The size of the receive buffer of `port` at the kernel's open.
fn size(port: u64) -> u64 {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == "4.93" && row.port == port)
        .expect("a row for the port")
        .rx_buffer_size
}

/// A thread 0 call of hypercall 165 for the interrupt status of `port`.
fn status_call(port: u64, pc: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..2].copy_from_slice(&[port, VIRTUAL_UART_PARAM_INTERRUPT_STATUS]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: GET_VIRTUAL_UART_PARAM,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(27_102),
    }
}

/// The state after these changes, in order.
fn state(changes: impl IntoIterator<Item = UartChange>) -> UartState {
    let mut uart = UartState::default();
    for change in changes {
        uart.apply(change);
    }
    uart
}

fn triggers(port: u64, tx_trigger: u64, rx_trigger: u64) -> UartChange {
    UartChange::PortParamsSet(VirtualUartPortParams {
        port,
        tx_trigger,
        rx_trigger,
        interrupt_mask: VIRTUAL_UART_STATUS_RX,
    })
}

fn received(port: u64, count: usize) -> UartChange {
    UartChange::ReceiveBufferSet(ReceivedBytes {
        port,
        bytes: vec![0xA5; count],
    })
}

fn unread(port: u64, count: usize) -> UartChange {
    UartChange::RequestBytesSent(PendingRequest::empty(port).with(&vec![0; count]))
}

fn at_4_93(port: u64, uart: &UartState) -> HypercallDisposition {
    get_param(&status_call(port, KERNEL_STATUS_SC), Some("4.93"), uart)
}

/// lv1 returns 0 with the status in r4, and the vector's entry zeroes r5
/// to r10 [OFW-4.93 lv1 a:0x2F3694-0x2F36A0] [OFW-4.93 lv1 a:0x0C08].
fn value(value: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [value, 0, 0, 0, 0, 0, 0]))
}

fn refused(what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: GET_VIRTUAL_UART_PARAM,
        unit: UnitId::new(1),
        pc: KERNEL_STATUS_SC,
        what,
    })
}

#[test]
fn a_service_port_with_no_traffic_has_the_connected_and_transmit_bits_at_any_pc() {
    for port in [SYSTEM_MANAGER_PORT, DISPATCHER_PORT] {
        for pc in [KERNEL_STATUS_SC, 0, KERNEL_STATUS_SC + 4] {
            assert_eq!(
                get_param(&status_call(port, pc), Some("4.93"), &UartState::default()),
                value(CONNECTED_TX),
                "port {port} pc {pc:#x}"
            );
        }
    }
}

/// lv1 sets the receive bit when the buffer size less the count is not
/// above the receive trigger [OFW-4.93 lv1 a:0x2F0D58-0x2F0D64].
#[test]
fn a_reply_that_brings_the_free_bytes_to_the_receive_trigger_adds_the_receive_bit() {
    let rx = size(DISPATCHER_PORT) - 0x48;
    let below = state([
        triggers(DISPATCHER_PORT, 0, rx),
        received(DISPATCHER_PORT, 0x47),
    ]);
    let at = state([
        triggers(DISPATCHER_PORT, 0, rx),
        received(DISPATCHER_PORT, 0x48),
    ]);
    assert_eq!(at_4_93(DISPATCHER_PORT, &below), value(CONNECTED_TX));
    assert_eq!(
        at_4_93(DISPATCHER_PORT, &at),
        value(CONNECTED_TX | VIRTUAL_UART_STATUS_RX)
    );
}

/// The peer holds the request bytes that the service did not read. So
/// CellGov knows the transmit bit only when that count is not above the
/// transmit trigger [OFW-4.93 lv1 a:0x2F0D48-0x2F0D54].
#[test]
fn unread_request_bytes_above_the_transmit_trigger_are_a_refusal() {
    let sent = 16;
    for tx in [0, sent as u64 - 1] {
        let uart = state([
            triggers(DISPATCHER_PORT, tx, 0),
            unread(DISPATCHER_PORT, sent),
        ]);
        assert_eq!(
            at_4_93(DISPATCHER_PORT, &uart),
            refused(UNREAD_REFUSAL),
            "tx trigger {tx}"
        );
    }
    for tx in [sent as u64, u64::MAX] {
        let uart = state([
            triggers(DISPATCHER_PORT, tx, 0),
            unread(DISPATCHER_PORT, sent),
        ]);
        assert_eq!(
            at_4_93(DISPATCHER_PORT, &uart),
            value(CONNECTED_TX),
            "tx trigger {tx}"
        );
    }
}

#[test]
fn the_status_of_a_kernel_port_that_is_not_a_service_port_is_a_refusal() {
    let what = "the interrupt status of a virtual UART port whose peer and traffic no source gives";
    assert_eq!(at_4_93(0, &UartState::default()), refused(what));
}

/// The kernel's interrupt function sends 165 with r4 = 7 and the port of
/// its record [OFW-4.93 lv2_kernel a:0x80000000000174CC-0x80000000000174E4].
/// A receive trigger of the buffer size, which 164 sets before the call,
/// adds the receive bit to an empty buffer's status.
#[test]
fn the_hypervisor_answers_the_kernel_s_status_call_from_the_group_s_state_and_keeps_it() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let storage = GuestMemory::new(0x1000);
    let rx_trigger = size(DISPATCHER_PORT);
    let mut set_rx_trigger = status_call(DISPATCHER_PORT, 0);
    set_rx_trigger.number = SET_VIRTUAL_UART_PARAM;
    set_rx_trigger.inputs[1..3].copy_from_slice(&[VIRTUAL_UART_PARAM_RX_TRIGGER, rx_trigger]);
    let HypercallDisposition::Answered(set) = hv.dispatch_with(&set_rx_trigger, &storage) else {
        panic!("the receive trigger call is not answered");
    };
    assert_eq!(set.r3, 0);
    let before = hv.groups().uart().clone();
    let answer = hv.dispatch_with(&status_call(DISPATCHER_PORT, KERNEL_STATUS_SC), &storage);
    assert_eq!(answer, value(CONNECTED_TX | VIRTUAL_UART_STATUS_RX));
    assert_eq!(hv.groups().uart(), &before);
}
