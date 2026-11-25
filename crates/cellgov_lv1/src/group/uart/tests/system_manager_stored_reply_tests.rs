use super::reply_tests::{
    kernel_reply, kernel_request, queued, request, rx_buffer_size, storage, write_with,
    KERNEL_READ_SC, KERNEL_SC, PORT,
};
use super::*;
use crate::answer::HypercallAnswer;
use crate::Hypervisor;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{READ_VIRTUAL_UART, WRITE_VIRTUAL_UART};

/// The 24 bytes that the system manager writes for the kernel's request
/// with payload byte 1 = 1 and tag 0 at the first GameOS boot after lv1
/// starts: the header of service id 0x0A with 8 payload bytes, then
/// payload byte 0 = 1 and seven 0 bytes. The count of stored bytes is 0,
/// so no byte follows; see
/// `cellgov_ps3_abi::lv1::uart::GAME_OS_SYSTEM_MANAGER_REPLIES`.
const STORED_REPLY: [u8; 24] = [
    0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// The bytes of the 0x0B reply that stay queued after the kernel reads
/// its 16-byte header.
fn rest_of_first_reply() -> Vec<u8> {
    kernel_reply()[16..].to_vec()
}

#[test]
fn the_kernel_s_second_request_queues_the_24_byte_reply_after_the_rest_of_the_first_reply() {
    let rest = rest_of_first_reply();
    assert_eq!(
        write_with(&rest, &kernel_request(1, 0)),
        queued(24, [&rest[..], &STORED_REPLY].concat())
    );
}

#[test]
fn the_stored_bytes_reply_header_carries_the_request_s_tag() {
    let mut reply = STORED_REPLY;
    reply[12..16].copy_from_slice(&[0x0A, 0x0B, 0x0C, 0x0D]);
    assert_eq!(
        write_with(&[], &kernel_request(1, 0x0A0B_0C0D)),
        queued(24, reply.to_vec())
    );
}

#[test]
fn a_stored_bytes_reply_past_the_free_bytes_of_the_receive_buffer_is_a_refusal() {
    let room = rx_buffer_size() - STORED_REPLY.len();
    assert_eq!(
        write_with(&vec![0; room], &kernel_request(1, 0)),
        queued(24, [vec![0; room], STORED_REPLY.to_vec()].concat())
    );
    assert_eq!(
        write_with(&vec![0; room + 1], &kernel_request(1, 0)),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: WRITE_VIRTUAL_UART,
            unit: UnitId::new(1),
            pc: KERNEL_SC,
            what: RECEIVE_BUFFER_REFUSAL,
        })
    );
}

/// A request of service id 0x0A in the shape of the 4.93 kernel's
/// `sys_sm_shutdown` path, with 4 bytes to store
/// [OFW-4.93 lv2_kernel a:0x8000000000264830-0x8000000000264858]:
/// - the header counts 8 + 4 payload bytes;
/// - payload byte 1 is 1, and the halfword at payload byte 2 is 4;
/// - the 4 bytes follow the 8 payload bytes.
///
/// The kernel stores no other payload byte, and this request holds 0 in
/// each of them.
#[test]
fn a_request_of_service_0x0a_that_stores_bytes_is_the_refusal_that_names_service_0x000a() {
    let mut bytes = vec![1, 0x10, 0, 0, 0, 0, 0, 12, 0, 0x0A, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&[0, 1, 0, 4, 0, 0, 0, 0, 0xAA, 0xBB, 0xCC, 0xDD]);
    assert_eq!(
        write_with(&[], &bytes),
        HypercallDisposition::Refused(HypercallRefusal::SystemManagerRequest {
            unit: UnitId::new(1),
            pc: KERNEL_SC,
            service: 0x000A,
        })
    );
}

/// The answer that `hv` gives to `r` with `bytes` in storage. A refusal
/// fails the test.
fn dispatch(hv: &mut Hypervisor, r: &HypercallRequest, bytes: &[u8]) -> HypercallAnswer {
    match hv.dispatch_with(r, &storage(bytes)) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal}"),
    }
}

/// The bytes that a kernel read of `size` bytes takes from the port.
fn read(hv: &mut Hypervisor, size: u64) -> Vec<u8> {
    let read_at = 0x0010_0000;
    let mut r = request(READ_VIRTUAL_UART, KERNEL_READ_SC, size);
    r.inputs[1] = read_at;
    let answer = dispatch(hv, &r, &[0]);
    assert_eq!(answer.outputs(), Some([size, 0, 0, 0, 0, 0, 0]));
    let [Effect::SharedWriteIntent { range, bytes, .. }] = answer.effects.as_slice() else {
        panic!("one write: {:?}", answer.effects);
    };
    assert_eq!((range.start().raw(), range.length()), (read_at, size));
    bytes.bytes().to_vec()
}

#[test]
fn the_hypervisor_answers_both_boot_requests_and_the_kernel_s_reads_take_both_replies_in_order() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let write = request(WRITE_VIRTUAL_UART, KERNEL_SC, 24);
    let first = dispatch(&mut hv, &write, &kernel_request(2, 0));
    assert_eq!(first.outputs(), Some([24, 0, 0, 0, 0, 0, 0]));
    assert_eq!(read(&mut hv, 16), kernel_reply()[..16]);
    let second = dispatch(&mut hv, &write, &kernel_request(1, 0));
    assert_eq!(second.outputs(), Some([24, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        hv.groups().uart().pending_request(PORT),
        PendingRequest::empty(PORT)
    );
    let mut taken = Vec::new();
    for size in [16, 16, 8] {
        taken.extend_from_slice(&read(&mut hv, size));
    }
    assert_eq!(taken, [&rest_of_first_reply()[..], &STORED_REPLY].concat());
    assert!(hv.groups().uart().received(PORT).is_empty());
}
