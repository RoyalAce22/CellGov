use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONSTRUCT_EVENT_RECEIVE_PORT;
use cellgov_time::GuestTicks;

/// The port id of the 4.93 kernel's first call: +0x40 of the record of
/// thread 0 in the capture `lv2_logical_ppe_record`.
const FIRST_PORT_ID: u64 = 3;

/// The 4.93 kernel's call: `sc 1` [OFW-4.93 lv2_kernel a:0x8000000000007d8c],
/// with r3 to r10 as hypercall 11 left them.
fn request() -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        inputs: [0; 8],
        pc: 0x8000_0000_0000_7d8c,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(52_118),
    }
}

fn constructed(id: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [id, 0, 0, 0, 0, 0, 0]);
    answer.push_change(InterruptChange::PortConstructed(EventReceivePort::new(id)));
    HypercallDisposition::Answered(answer)
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

fn holding(id: u64) -> EventReceivePortState {
    let mut ports = EventReceivePortState::default();
    ports.construct(EventReceivePort::new(id));
    ports
}

#[test]
fn the_kernels_first_call_gets_the_first_port_id_in_r4_and_r5_to_r10_cleared() {
    assert_eq!(
        construct_port(&request(), Some("4.93"), &EventReceivePortState::default()),
        constructed(FIRST_PORT_ID)
    );
}

#[test]
fn the_answer_reads_no_input_register() {
    let mut r = request();
    r.inputs = [0xA5A5_A5A5_A5A5_A5A5; 8];
    assert_eq!(
        construct_port(&r, Some("4.93"), &EventReceivePortState::default()),
        constructed(FIRST_PORT_ID)
    );
}

#[test]
fn a_second_record_call_on_one_thread_is_refused() {
    let r = request();
    assert_eq!(
        construct_port(&r, Some("4.93"), &holding(FIRST_PORT_ID)),
        refused(
            &r,
            "an event receive port call whose port id the hypervisor already holds"
        )
    );
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_refused() {
    let r = request();
    for firmware in [None, Some("3.70"), Some("4.92")] {
        assert_eq!(
            construct_port(&r, firmware, &EventReceivePortState::default()),
            refused(
                &r,
                "an event receive port call at a firmware whose lv1 handler is not read"
            ),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_keeps_the_port_and_hashes_it() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let empty = hv.sync_term();
    assert_eq!(hv.dispatch(&request()), constructed(FIRST_PORT_ID));
    assert_eq!(hv.event_receive_ports().ports().len(), 1);
    assert_eq!(
        hv.event_receive_ports().port(FIRST_PORT_ID),
        Some(&EventReceivePort::new(FIRST_PORT_ID))
    );
    assert_ne!(hv.sync_term(), empty);
    assert!(hv.witnesses().is_empty());
    assert_eq!(hv.refusal(), None);
}

#[test]
fn a_refused_call_keeps_no_port() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.92");
    let r = request();
    assert!(matches!(hv.dispatch(&r), HypercallDisposition::Refused(_)));
    assert!(hv.event_receive_ports().ports().is_empty());
    assert_eq!(hv.sync_term(), Hypervisor::new().sync_term());
}

#[test]
fn a_refused_second_call_leaves_the_first_port() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let r = request();
    hv.dispatch(&r);
    let one = hv.sync_term();
    assert!(matches!(hv.dispatch(&r), HypercallDisposition::Refused(_)));
    assert_eq!(hv.event_receive_ports(), &holding(FIRST_PORT_ID));
    assert_eq!(hv.sync_term(), one);
}
