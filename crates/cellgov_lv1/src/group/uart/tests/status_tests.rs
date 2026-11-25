use super::*;
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::uart::set_param::set_param;
use crate::group::uart::{UartChange, UartState};
use crate::request::HypercallRequest;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::SET_VIRTUAL_UART_PARAM;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call that adds bits to a port's mask
/// [OFW-4.93 lv2_kernel a:0x8000000000016ac8].
const KERNEL_ADD_MASK_SC: u64 = 0x8000_0000_0001_6ac8;

/// The service ports and their receive buffer sizes in the capture
/// `lv2_vuart_port_params`.
const SERVICE_PORTS: [(u64, u64); 2] = [(2, 0x800), (10, 0x1000)];

const TX_RAISE: &str = "the transmit bit 1 of a virtual UART interrupt mask on a port whose peer \
                        holds no more bytes than the transmit trigger, with which lv1 raises the \
                        port's interrupt";

const RX_RAISE: &str = "the receive bit 2 of a virtual UART interrupt mask on a port whose \
                        receive buffer has no more free bytes than the receive trigger, with \
                        which lv1 raises the port's interrupt";

const RX_UNKNOWN: &str = "the receive bit 2 of a virtual UART interrupt mask on a port whose \
                          peer and traffic no source gives";

fn params(port: u64, tx: u64, rx: u64, mask: u64) -> VirtualUartPortParams {
    VirtualUartPortParams {
        port,
        tx_trigger: tx,
        rx_trigger: rx,
        interrupt_mask: mask,
    }
}

/// A thread 0 call of hypercall 164 at the kernel's `sc` that adds bits
/// to the mask.
fn request(port: u64, param: u64, value: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[port, param, value]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: SET_VIRTUAL_UART_PARAM,
        inputs,
        pc: KERNEL_ADD_MASK_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_552),
    }
}

/// lv1 returns 0 [OFW-4.93 lv1 a:0x2F3DC8-0x2F3DCC], and the vector's
/// entry zeroes r4 to r10 [OFW-4.93 lv1 a:0x0C08].
fn set(params: VirtualUartPortParams) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(UartChange::PortParamsSet(params));
    HypercallDisposition::Answered(answer)
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: SET_VIRTUAL_UART_PARAM,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// The kernel's open sets each trigger to the buffer size less 1, then
/// the mask 0 [OFW-4.93 lv2_kernel a:0x80000000000177bc-0x80000000000177ec].
/// After an empty read it adds 2 to the mask
/// [OFW-4.93 lv2_kernel a:0x8000000000016bbc-0x8000000000016bc4]
/// [OFW-4.93 lv2_kernel a:0x8000000000016aa8-0x8000000000016ac8].
#[test]
fn the_kernel_s_mask_2_on_each_service_port_after_its_open_raises_nothing_and_is_kept() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let storage = GuestMemory::new(0x1000);
    for (port, size) in SERVICE_PORTS {
        for (param, value) in [(0, size - 1), (1, size - 1), (2, 0), (2, 2)] {
            let answer = hv.dispatch_with(&request(port, param, value), &storage);
            assert!(
                matches!(answer, HypercallDisposition::Answered(ref a) if a.r3 == 0),
                "port {port} param {param}: {answer:?}"
            );
        }
        assert_eq!(
            hv.groups().uart().port_params(port),
            params(port, size - 1, size - 1, 2),
            "port {port}"
        );
    }
}

#[test]
fn mask_2_on_an_empty_service_port_raises_only_once_the_receive_trigger_reaches_the_size() {
    for (port, size) in SERVICE_PORTS {
        for rx in [0, 1, size - 1] {
            assert_eq!(
                mask_refusal("4.93", &params(port, 0, rx, 2)),
                None,
                "port {port} rx {rx:#x}"
            );
        }
        for rx in [size, size + 1, u64::MAX] {
            assert_eq!(
                mask_refusal("4.93", &params(port, 0, rx, 2)),
                Some(RX_RAISE),
                "port {port} rx {rx:#x}"
            );
        }
    }
}

/// The traffic gives the peer's receive buffer no byte, which is not above
/// any transmit trigger.
#[test]
fn the_transmit_bit_on_a_service_port_raises_for_each_transmit_trigger() {
    for (port, size) in SERVICE_PORTS {
        for mask in [1, 3, 5, 7] {
            for tx in [0, size - 1, u64::MAX] {
                assert_eq!(
                    mask_refusal("4.93", &params(port, tx, size - 1, mask)),
                    Some(TX_RAISE),
                    "port {port} mask {mask} tx {tx:#x}"
                );
            }
        }
    }
}

/// A service port's peer has a reference, so the status has no
/// disconnect bit.
#[test]
fn the_disconnect_bit_on_a_service_port_raises_nothing() {
    for (port, size) in SERVICE_PORTS {
        for mask in [4, 6] {
            assert_eq!(
                mask_refusal("4.93", &params(port, 0, size - 1, mask)),
                None,
                "port {port} mask {mask}"
            );
        }
    }
}

#[test]
fn a_mask_of_0_raises_nothing_on_any_port_for_any_trigger() {
    for port in [0, 2, 10] {
        for trigger in [0, u64::MAX] {
            assert_eq!(
                mask_refusal("4.93", &params(port, trigger, trigger, 0)),
                None,
                "port {port} trigger {trigger:#x}"
            );
        }
    }
}

/// No source gives the service ports at another firmware.
#[test]
fn mask_2_on_a_service_port_at_a_firmware_with_no_port_rows_is_a_traffic_refusal() {
    assert_eq!(
        mask_refusal("4.92", &params(10, 0, 0xfff, 2)),
        Some(RX_UNKNOWN)
    );
}

/// lv1 computes the status after a trigger call too
/// [OFW-4.93 lv1 a:0x2F3D70-0x2F3DC4].
#[test]
fn a_receive_trigger_call_on_a_service_port_with_mask_2_raises_only_at_the_size() {
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(params(10, 0xfff, 0xfff, 2)));
    let r = request(10, 1, 0x1000);
    assert_eq!(set_param(&r, Some("4.93"), &uart), refused(&r, RX_RAISE));
    assert_eq!(
        set_param(&request(10, 1, 0xffe), Some("4.93"), &uart),
        set(params(10, 0xfff, 0xffe, 2))
    );
}
