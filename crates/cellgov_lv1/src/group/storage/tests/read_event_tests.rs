use super::*;
use crate::group::interrupt::InterruptChange;
use crate::group::state::StateChange;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::{
    CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, CONSTRUCT_EVENT_RECEIVE_PORT, OPEN_DEVICE,
    STORAGE_GET_ASYNC_STATUS, STORAGE_WRITE,
};
use cellgov_ps3_abi::lv1::storage::NOTIFY_WRITE_COMMAND;
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's `open_device` of the storage bus
/// [OFW-4.93 lv2_kernel a:0x80000000000AE110].
const KERNEL_OPEN_SC: u64 = 0x8000_0000_000A_E110;

/// The `sc` of the port setup's `construct_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const KERNEL_PORT_SC: u64 = 0x8000_0000_0009_6494;

/// The `sc` of the port setup's `connect_interrupt_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x80000000000964CC].
const KERNEL_CONNECT_SC: u64 = 0x8000_0000_0009_64CC;

/// The `sc` of the kernel's notification write
/// [OFW-4.93 lv2_kernel a:0x80000000000AE250].
const KERNEL_WRITE_SC: u64 = 0x8000_0000_000A_E250;

/// The `sc` of the kernel's notification read
/// [OFW-4.93 lv2_kernel a:0x80000000000AE408].
const KERNEL_READ_SC: u64 = 0x8000_0000_000A_E408;

/// The addresses of the write's and the read's buffers, in the real mode
/// region.
const WRITE_BUFFER: u64 = 0x1_0000;
const READ_BUFFER: u64 = 0x1_1000;

/// The event mask that the kernel stores at +8 of its write's buffer
/// [OFW-4.93 lv2_kernel a:0x80000000000AE1F4-0x80000000000AE200].
const KERNEL_MASK: u64 = 2;

fn call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(34_296),
    }
}

fn answered(
    hv: &mut Hypervisor,
    request: &HypercallRequest,
    memory: &GuestMemory,
) -> HypercallAnswer {
    match hv.dispatch_with(request, memory) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    }
}

/// The write's buffer with the command 0 and [`KERNEL_MASK`].
fn kernel_memory() -> GuestMemory {
    let mut memory = GuestMemory::new(0x2_0000);
    let head = ByteRange::new(GuestAddr::new(WRITE_BUFFER), 16).expect("the buffer head");
    let mut bytes = NOTIFY_WRITE_COMMAND.to_be_bytes().to_vec();
    bytes.extend_from_slice(&KERNEL_MASK.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    memory
}

/// The test connects the port after the write, so the write's event
/// leaves the port idle. The report's event is then the first event on
/// the port.
#[test]
fn a_device_report_sends_an_event_on_the_connected_storage_port() {
    let memory = kernel_memory();
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = call(OPEN_DEVICE, KERNEL_OPEN_SC, &[4, u64::MAX, 0]);
    assert_eq!(answered(&mut hv, &open, &memory).r3, 0, "open");
    let write = call(
        STORAGE_WRITE,
        KERNEL_WRITE_SC,
        &[u64::MAX, 0, 0, 1, 0, WRITE_BUFFER],
    );
    assert_eq!(answered(&mut hv, &write, &memory).r3, 0, "write");
    let status = call(STORAGE_GET_ASYNC_STATUS, KERNEL_WRITE_SC, &[u64::MAX]);
    assert_eq!(answered(&mut hv, &status, &memory).r3, 0, "status");
    let made = answered(
        &mut hv,
        &call(CONSTRUCT_EVENT_RECEIVE_PORT, KERNEL_PORT_SC, &[]),
        &memory,
    );
    let port = made.outputs().expect("a table return")[0];
    let connect = call(
        CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
        KERNEL_CONNECT_SC,
        &[4, u64::MAX, port, 0],
    );
    assert_eq!(answered(&mut hv, &connect, &memory).r3, 0, "connect");
    assert!(!hv.groups().interrupt().ports().is_pending(port));
    let read = call(
        STORAGE_READ,
        KERNEL_READ_SC,
        &[u64::MAX, 0, 0, 1, 0, READ_BUFFER],
    );
    let answer = answered(&mut hv, &read, &memory);
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([1, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        answer.changes,
        vec![
            StateChange::from(StorageChange::DeviceRegistered(0)),
            StateChange::from(StorageChange::NotifyDeviceReported),
            StateChange::from(StorageChange::NotifyRequested(NotifyRequest {
                tag: 1,
                completion: Some(NOTIFY_READ_COMPLETION_STATUS),
            })),
            StateChange::from(InterruptChange::EventPending(port)),
        ]
    );
    assert!(hv.groups().interrupt().ports().is_pending(port));
}
