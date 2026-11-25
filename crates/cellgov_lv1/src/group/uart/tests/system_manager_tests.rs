use super::*;
use crate::answer::HypercallAnswer;
use crate::group::interrupt::InterruptGroupState;
use crate::group::memory::region::MemoryState;
use crate::group::uart::write::write as write_virtual_uart;
use crate::group::uart::UartState;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ps3_abi::lv1::call::WRITE_VIRTUAL_UART;
use cellgov_ps3_abi::lv1::uart::GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's write
/// [OFW-4.93 lv2_kernel a:0x8000000000016fd8].
const KERNEL_SC: u64 = 0x8000_0000_0001_6fd8;

/// The system manager's port [OFW-4.93 lv1 a:0x44CF04-0x44CF0C].
const PORT: u64 = 2;

/// A buffer address in the real mode region, away from its end.
const BUFFER: u64 = 0x0049_7940;

/// A header with the version 1, the size 16, `payload` payload bytes,
/// the service id `service` and the tag 0, as the 4.93 kernel builds it
/// [OFW-4.93 lv2_kernel a:0x80000000002646e0-0x8000000000264718].
const fn header(payload: u32, service: u16) -> [u8; 16] {
    let p = payload.to_be_bytes();
    let s = service.to_be_bytes();
    [
        1, 0x10, 0, 0, p[0], p[1], p[2], p[3], s[0], s[1], 0, 0, 0, 0, 0, 0,
    ]
}

/// The 24 bytes of the 4.93 kernel's first write to the port in
/// CellGov's boot: the header of service id 9 with 8 payload bytes, then
/// the payload bytes 1 and 2 and six 0 bytes. The kernel's function
/// stores the first two payload bytes
/// [OFW-4.93 lv2_kernel a:0x80000000002646ec]
/// [OFW-4.93 lv2_kernel a:0x8000000000264714].
fn kernel_request() -> Vec<u8> {
    let mut bytes = header(8, GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE).to_vec();
    bytes.extend_from_slice(&[1, 2, 0, 0, 0, 0, 0, 0]);
    bytes
}

fn request(size: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[PORT, BUFFER, size]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_VIRTUAL_UART,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(27_683),
    }
}

/// A real mode storage of 8 MB with `bytes` at [`BUFFER`].
fn storage(bytes: &[u8]) -> GuestMemory {
    let mut storage = GuestMemory::from_regions(vec![Region::new(
        0,
        0x80_0000,
        "real mode",
        PageSize::Page64K,
    )])
    .expect("one region");
    let range = ByteRange::new(GuestAddr::new(BUFFER), bytes.len() as u64).expect("a range");
    storage.apply_commit(range, bytes).expect("held");
    storage
}

/// The answer to a write of `bytes` at [`BUFFER`] after the port's request
/// `pending`.
fn write_after(pending: PendingRequest, bytes: &[u8]) -> HypercallDisposition {
    let mut uart = UartState::default();
    uart.apply(UartChange::RequestBytesSent(pending));
    write_virtual_uart(
        &request(bytes.len() as u64),
        &WriteInput {
            firmware: Some("4.93"),
            memory: &MemoryState::default(),
            uart: &uart,
            interrupt: &InterruptGroupState::default(),
            storage: &storage(bytes),
        },
    )
}

fn first_write(bytes: &[u8]) -> HypercallDisposition {
    write_after(PendingRequest::empty(PORT), bytes)
}

/// The request after the first `bytes.len()` bytes, which are at most 24,
/// built without `PendingRequest::with`.
fn sent(bytes: &[u8]) -> PendingRequest {
    let mut prefix = [0u8; 24];
    prefix[..bytes.len()].copy_from_slice(bytes);
    PendingRequest {
        port: PORT,
        sent: bytes.len() as u64,
        prefix,
    }
}

fn written(size: u64, left: PendingRequest) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [size, 0, 0, 0, 0, 0, 0]);
    answer.push_change(UartChange::RequestBytesSent(left));
    HypercallDisposition::Answered(answer)
}

fn refused(what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: WRITE_VIRTUAL_UART,
        unit: UnitId::new(1),
        pc: KERNEL_SC,
        what,
    })
}

fn refused_for(service: u16) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::SystemManagerRequest {
        unit: UnitId::new(1),
        pc: KERNEL_SC,
        service,
    })
}

#[test]
fn the_kernel_s_request_of_service_9_in_one_write_is_the_refusal_that_names_service_9() {
    assert_eq!(first_write(&kernel_request()), refused_for(9));
}

#[test]
fn the_refusal_names_the_whole_service_id_halfword() {
    let mut bytes = header(8, 0x0109).to_vec();
    bytes.extend_from_slice(&[1, 2, 0, 0, 0, 0, 0, 0]);
    assert_eq!(first_write(&bytes), refused_for(0x0109));
}

#[test]
fn the_refusal_text_and_number_name_hypercall_163_and_the_service_id() {
    let refusal = HypercallRefusal::SystemManagerRequest {
        unit: UnitId::new(0),
        pc: KERNEL_SC,
        service: 9,
    };
    assert_eq!(refusal.number(), Some(163));
    assert_eq!(
        refusal.to_string(),
        "hypercall 163 from UnitId(0) at pc 0x8000000000016fd8: CellGov does not hold the inputs \
         of the system manager's reply to a request of service id 0x0009"
    );
}

#[test]
fn a_write_that_ends_inside_the_header_is_a_refusal() {
    let request = kernel_request();
    for size in [1, 8, 15] {
        assert_eq!(
            first_write(&request[..size]),
            refused(HEADER_SPLIT_REFUSAL),
            "{size}"
        );
    }
}

#[test]
fn a_write_of_the_header_alone_is_answered_and_keeps_the_header() {
    let request = kernel_request();
    assert_eq!(
        first_write(&request[..16]),
        written(16, sent(&request[..16]))
    );
}

#[test]
fn a_payload_sent_in_two_writes_is_answered_once_and_completes_at_its_last_byte() {
    let request = kernel_request();
    let after_first = sent(&request[..20]);
    assert_eq!(first_write(&request[..20]), written(20, after_first));
    assert_eq!(write_after(after_first, &request[20..]), refused_for(9));
}

#[test]
fn a_header_that_counts_0_payload_bytes_completes_with_no_request_left_and_no_reply() {
    let bytes = header(0, GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE);
    assert_eq!(
        first_write(&bytes),
        written(16, PendingRequest::empty(PORT))
    );
}

#[test]
fn a_negative_payload_count_completes_the_request_with_its_header() {
    for count in [0x8000_0000, 0xFFFF_FFFF] {
        let bytes = header(count, GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE);
        assert_eq!(first_write(&bytes), refused_for(9), "{count:#x}");
    }
}

#[test]
fn a_write_past_the_end_of_a_request_is_a_refusal() {
    let mut no_payload = header(0, GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE).to_vec();
    no_payload.extend_from_slice(&[0; 8]);
    let mut one_more = kernel_request();
    one_more.push(0);
    for bytes in [no_payload, one_more] {
        assert_eq!(
            first_write(&bytes),
            refused(OVERRUN_REFUSAL),
            "{}",
            bytes.len()
        );
    }
}

/// The peer's buffer is 2048 bytes [OFW-4.93 lv1 a:0x44CF04-0x44CF24], so
/// the largest request that it holds counts 2032 payload bytes.
#[test]
fn a_request_of_2032_payload_bytes_is_open_one_byte_short_and_complete_at_2048_bytes() {
    let head = header(2032, GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE);
    let mut bytes = head.to_vec();
    bytes.resize(2047, 0);
    let open = PendingRequest {
        sent: 2047,
        ..sent(&bytes[..24])
    };
    assert_eq!(first_write(&bytes), written(2047, open));
    assert_eq!(write_after(open, &[0]), refused_for(9));
}
