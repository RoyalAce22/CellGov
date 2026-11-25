use super::*;
use crate::answer::HypercallAnswer;
use crate::group::interrupt::InterruptGroupState;
use crate::group::memory::region::MemoryState;
use crate::group::uart::write::write as write_virtual_uart;
use crate::group::uart::{UartState, VirtualUartPortParams, VirtualUartShadowWrite};
use crate::Hypervisor;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ps3_abi::lv1::call::{READ_VIRTUAL_UART, WRITE_VIRTUAL_UART};
use cellgov_ps3_abi::lv1::uart::{
    GAME_OS_VIRTUAL_UART_PORTS, SYSTEM_MANAGER_REPLY_CAPTURE, VIRTUAL_UART_STATUS_RX,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's write
/// [OFW-4.93 lv2_kernel a:0x8000000000016fd8].
pub(super) const KERNEL_SC: u64 = 0x8000_0000_0001_6fd8;

/// The `sc` of the 4.93 kernel's read
/// [OFW-4.93 lv2_kernel a:0x8000000000016b7c].
pub(super) const KERNEL_READ_SC: u64 = 0x8000_0000_0001_6b7c;

/// The system manager's port [OFW-4.93 lv1 a:0x44CF04-0x44CF0C].
pub(super) const PORT: u64 = 2;

/// A buffer address in the real mode region, away from its end.
const BUFFER: u64 = 0x0049_7940;

/// The 24 bytes of the 4.93 kernel's request with the tag `tag`: the
/// header of service id 9 with 8 payload bytes, then payload byte 0 = 1,
/// payload byte 1 = `selector` and six 0 bytes
/// [OFW-4.93 lv2_kernel a:0x80000000002646e0-0x8000000000264718].
pub(super) fn kernel_request(selector: u8, tag: u32) -> Vec<u8> {
    let t = tag.to_be_bytes();
    vec![
        1, 0x10, 0, 0, 0, 0, 0, 8, 0, 9, 0, 0, t[0], t[1], t[2], t[3], 1, selector, 0, 0, 0, 0, 0,
        0,
    ]
}

/// The doublewords of the committed file `reply.tsv` of the console
/// capture `lv2_system_manager_reply`, as (field, value). Both reads of
/// each doubleword must agree.
fn captured_words() -> Vec<(String, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(SYSTEM_MANAGER_REPLY_CAPTURE)
        .join("peek/cech20-cex-493/reply.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 6, "{name}: {line}");
            assert_eq!(row[4], "yes", "{name}: {line}");
            let value = u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
                .unwrap_or_else(|e| panic!("{name}: {line}: {e}"));
            (row[1].to_string(), value)
        })
        .collect()
}

/// The 32 bytes that the system manager writes for the kernel's request
/// with tag 0.
///
/// The header is the header of service id 0x0B with 16 payload bytes
/// [OFW-4.93 lv1 a:0x44BA78-0x44BAA8] [OFW-4.93 lv1 a:0x44DF18-0x44DF24].
/// The payload is read from the console capture: the kernel keeps payload
/// bytes 1 and 2 at +2032 of its object, the doubleword at payload byte 8
/// at +2040, and the word at payload byte 4 at +2048
/// [OFW-4.93 lv2_kernel a:0x8000000000261a94-0x8000000000261ab8]. lv1
/// sets payload byte 0 to 2 and byte 3 to 0 [OFW-4.93 lv1 a:0x44BAF4-0x44BB14].
pub(super) fn kernel_reply() -> [u8; 32] {
    let words = captured_words();
    let word = |field: &str| {
        words
            .iter()
            .find(|(f, _)| f == field)
            .map(|&(_, value)| value.to_be_bytes())
            .unwrap_or_else(|| panic!("reply.tsv: no row for {field}"))
    };
    let mut reply = [0u8; 32];
    reply[..16].copy_from_slice(&[1, 0x10, 0, 0, 0, 0, 0, 0x10, 0, 0x0B, 0, 0, 0, 0, 0, 0]);
    reply[16] = 2;
    reply[17..19].copy_from_slice(&word("reply2032")[..2]);
    reply[20..24].copy_from_slice(&word("reply2048")[..4]);
    reply[24..].copy_from_slice(&word("reply2040"));
    reply
}

pub(super) fn request(number: u64, pc: u64, size: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[PORT, BUFFER, size]);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(27_683),
    }
}

/// A real mode storage of 8 MB with `bytes` at [`BUFFER`].
pub(super) fn storage(bytes: &[u8]) -> GuestMemory {
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

/// The answer to a write of `bytes` with `received` in the port's receive
/// buffer.
pub(super) fn write_with(received: &[u8], bytes: &[u8]) -> HypercallDisposition {
    let mut uart = UartState::default();
    uart.apply(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: PORT,
        bytes: received.to_vec(),
    }));
    write_from(&uart, bytes)
}

/// The answer to a write of `bytes` with the virtual UART state `uart`.
fn write_from(uart: &UartState, bytes: &[u8]) -> HypercallDisposition {
    write_virtual_uart(
        &request(WRITE_VIRTUAL_UART, KERNEL_SC, bytes.len() as u64),
        &WriteInput {
            firmware: Some("4.93"),
            memory: &MemoryState::default(),
            uart,
            interrupt: &InterruptGroupState::default(),
            storage: &storage(bytes),
        },
    )
}

pub(super) fn queued(written: u64, bytes: Vec<u8>) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [written, 0, 0, 0, 0, 0, 0]);
    answer.push_change(UartChange::RequestBytesSent(PendingRequest::empty(PORT)));
    answer.push_change(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: PORT,
        bytes,
    }));
    HypercallDisposition::Answered(answer)
}

/// The size of the receive buffer of [`PORT`] at the kernel's open.
pub(super) fn rx_buffer_size() -> usize {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == "4.93" && row.port == PORT)
        .expect("a row for the port")
        .rx_buffer_size as usize
}

#[test]
fn the_kernel_s_request_with_payload_byte_1_2_writes_24_bytes_and_queues_the_32_byte_reply() {
    assert_eq!(
        write_with(&[], &kernel_request(2, 0)),
        queued(24, kernel_reply().to_vec())
    );
}

#[test]
fn the_reply_header_carries_the_request_s_tag() {
    let mut reply = kernel_reply();
    reply[12..16].copy_from_slice(&[0x0A, 0x0B, 0x0C, 0x0D]);
    assert_eq!(
        write_with(&[], &kernel_request(2, 0x0A0B_0C0D)),
        queued(24, reply.to_vec())
    );
}

#[test]
fn the_reply_follows_the_bytes_that_the_receive_buffer_holds() {
    let expected = [&[0xEE; 3][..], &kernel_reply()].concat();
    assert_eq!(
        write_with(&[0xEE; 3], &kernel_request(2, 0)),
        queued(24, expected)
    );
}

#[test]
fn a_reply_past_the_free_bytes_of_the_receive_buffer_is_a_refusal() {
    let room = rx_buffer_size() - kernel_reply().len();
    assert_eq!(
        write_with(&vec![0; room], &kernel_request(2, 0)),
        queued(24, [vec![0; room], kernel_reply().to_vec()].concat())
    );
    assert_eq!(
        write_with(&vec![0; room + 1], &kernel_request(2, 0)),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: WRITE_VIRTUAL_UART,
            unit: UnitId::new(1),
            pc: KERNEL_SC,
            what: RECEIVE_BUFFER_REFUSAL,
        })
    );
}

#[test]
fn a_request_that_a_second_write_completes_writes_that_write_s_bytes_and_queues_the_reply() {
    let bytes = kernel_request(2, 0);
    let mut uart = UartState::default();
    uart.apply(UartChange::RequestBytesSent(
        PendingRequest::empty(PORT).with(&bytes[..17]),
    ));
    assert_eq!(
        write_from(&uart, &bytes[17..]),
        queued(7, kernel_reply().to_vec())
    );
}

/// The port's values with the receive trigger `rx_trigger`, the receive
/// bit as the mask, and the transmit trigger that the kernel's open sets:
/// the buffer size less 1 [OFW-4.93 lv2_kernel a:0x80000000000177bc-0x80000000000177ec].
fn rx_params(rx_trigger: u64) -> VirtualUartPortParams {
    VirtualUartPortParams {
        port: PORT,
        tx_trigger: rx_buffer_size() as u64 - 1,
        rx_trigger,
        interrupt_mask: VIRTUAL_UART_STATUS_RX,
    }
}

/// The status has the receive bit only when the free bytes are not above
/// the receive trigger [OFW-4.93 lv1 a:0x2F2208-0x2F2210]. Port 2's bit
/// is bit 63 - 2 of shadow doubleword 0 [OFW-4.93 lv1 a:0x2F2214-0x2F2248].
#[test]
fn the_reply_raises_the_port_s_shadow_bit_only_once_the_free_bytes_reach_the_receive_trigger() {
    let free = (rx_buffer_size() - kernel_reply().len()) as u64;
    let write_under = |rx_trigger: u64| {
        let mut uart = UartState::default();
        uart.apply(UartChange::PortParamsSet(rx_params(rx_trigger)));
        write_from(&uart, &kernel_request(2, 0))
    };
    assert_eq!(write_under(free - 1), queued(24, kernel_reply().to_vec()));
    let HypercallDisposition::Answered(mut raised) = queued(24, kernel_reply().to_vec()) else {
        panic!("an answer");
    };
    raised.push_change(UartChange::ShadowSet(VirtualUartShadowWrite {
        index: 0,
        value: 1 << 61,
    }));
    assert_eq!(write_under(free), HypercallDisposition::Answered(raised));
}

#[test]
fn a_request_with_payload_byte_1_3_is_the_refusal_that_names_service_9() {
    assert_eq!(
        write_with(&[], &kernel_request(3, 0)),
        HypercallDisposition::Refused(HypercallRefusal::SystemManagerRequest {
            unit: UnitId::new(1),
            pc: KERNEL_SC,
            service: 9,
        })
    );
}

#[test]
fn the_hypervisor_queues_the_reply_and_the_kernel_s_reads_take_it_in_order() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let read_at = 0x0010_0000;
    let storage = storage(&kernel_request(2, 0));
    let dispatch = |hv: &mut Hypervisor, r: &HypercallRequest| match hv.dispatch_with(r, &storage) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal}"),
    };
    assert_eq!(
        dispatch(&mut hv, &request(WRITE_VIRTUAL_UART, KERNEL_SC, 24)).outputs(),
        Some([24, 0, 0, 0, 0, 0, 0])
    );
    assert_eq!(
        hv.groups().uart().pending_request(PORT),
        PendingRequest::empty(PORT)
    );
    assert_eq!(hv.groups().uart().received(PORT), kernel_reply());
    let mut taken = Vec::new();
    for size in [16, 16] {
        let mut read = request(READ_VIRTUAL_UART, KERNEL_READ_SC, size);
        read.inputs[1] = read_at;
        let answer = dispatch(&mut hv, &read);
        assert_eq!(answer.outputs(), Some([size, 0, 0, 0, 0, 0, 0]));
        let [Effect::SharedWriteIntent { range, bytes, .. }] = answer.effects.as_slice() else {
            panic!("one write: {:?}", answer.effects);
        };
        assert_eq!((range.start().raw(), range.length()), (read_at, size));
        taken.extend_from_slice(bytes.bytes());
    }
    assert_eq!(taken, kernel_reply());
    assert!(hv.groups().uart().received(PORT).is_empty());
}
