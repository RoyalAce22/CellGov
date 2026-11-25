use super::*;
use crate::group::interrupt::InterruptGroupState;
use crate::group::uart::VirtualUartPortParams;
use crate::Hypervisor;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv1::call::READ_VIRTUAL_UART;
use cellgov_ps3_abi::lv1::uart::{
    game_os_dispatcher_reply, GAME_OS_VIRTUAL_UART_PORTS, VIRTUAL_UART_PORT_MASK_CAPTURE,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's write
/// [OFW-4.93 lv2_kernel a:0x8000000000016fd8].
const KERNEL_SC: u64 = 0x8000_0000_0001_6fd8;

/// The `sc` of the 4.93 kernel's read
/// [OFW-4.93 lv2_kernel a:0x8000000000016b7c].
const KERNEL_READ_SC: u64 = 0x8000_0000_0001_6b7c;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// The address of the kernel's header, in the real mode region.
const HEADER_ADDRESS: u64 = 0x0036_0070;

/// The address of the kernel's request bytes in CellGov's boot, in the
/// real mode region.
const REQUEST_ADDRESS: u64 = 0x0049_7940;

/// The header that the 4.93 kernel sends first in CellGov's boot: tag 0,
/// service 0x19000, 0x38 request bytes and 0x38 reply bytes
/// [OFW-4.93 lv2_kernel a:0x80000000002121f4].
const KERNEL_HEADER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x90, 0x00, 0x00, 0x00, 0x00, 0x38, 0x00, 0x00, 0x00, 0x38,
];

/// The kernel's 0x38 request bytes: the request id and the service id,
/// then 0 to +32, then the argument that the kernel's first caller gives
/// [OFW-4.93 lv2_kernel a:0x8000000000212fb4].
fn kernel_request() -> Vec<u8> {
    let mut bytes = vec![0u8; 0x38];
    bytes[..8].copy_from_slice(&0x19002u64.to_be_bytes());
    bytes[8..16].copy_from_slice(&0x19000u64.to_be_bytes());
    bytes[32..40].copy_from_slice(&0x1050_0000_0300_0001u64.to_be_bytes());
    bytes
}

/// The reply that the dispatcher manager writes for [`kernel_request`]:
/// the reply header, then the request bytes with the status at +16 and
/// the data bytes at +40.
fn kernel_reply() -> Vec<u8> {
    let row = game_os_dispatcher_reply("4.93", 0x19000, 0x19002).expect("a source gives the reply");
    let mut body = kernel_request();
    body[16..20].copy_from_slice(&row.status.to_be_bytes());
    body[40..56].copy_from_slice(&row.data);
    let header = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x38];
    [&header[..], &body].concat()
}

/// The size of the receive buffer of [`PORT`] at the kernel's open.
fn rx_buffer_size() -> u64 {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == "4.93" && row.port == PORT)
        .expect("a row for the port")
        .rx_buffer_size
}

/// The interrupt mask that the 4.93 kernel keeps for [`PORT`].
///
/// The value is the mask of [`PORT`]'s record in the committed file of
/// the capture `lv2_vuart_port_mask`.
fn kernel_mask() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(VIRTUAL_UART_PORT_MASK_CAPTURE)
        .join("peek/cech20-cex-493/vuart_masks.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let rows: Vec<(&str, u64)> = lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 6, "{name}: {line}");
            assert_eq!(row[4], "yes", "{name}: {line}");
            let value = u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
                .unwrap_or_else(|e| panic!("{name}: {line}: {e}"));
            (row[1], value)
        })
        .collect();
    let value = |field: &str| {
        rows.iter()
            .find(|(f, _)| *f == field)
            .map(|&(_, value)| value)
    };
    let record = (0..3)
        .find(|n| value(&format!("record{n}_port")) == Some(PORT))
        .unwrap_or_else(|| panic!("{name}: no record of port {PORT}"));
    value(&format!("record{record}_mask"))
        .unwrap_or_else(|| panic!("{name}: no record{record}_mask"))
}

fn request(number: u64, pc: u64, address: u64, size: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[PORT, address, size]);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_802),
    }
}

fn write_request(address: u64, size: u64) -> HypercallRequest {
    request(WRITE_VIRTUAL_UART, KERNEL_SC, address, size)
}

/// The real mode storage of 8 MB, with `bytes` at each real address.
fn storage(bytes: &[(u64, &[u8])]) -> GuestMemory {
    let mut storage = GuestMemory::from_regions(vec![Region::new(
        0,
        0x80_0000,
        "real mode",
        PageSize::Page64K,
    )])
    .expect("one region");
    for &(real, data) in bytes {
        let range = ByteRange::new(GuestAddr::new(real), data.len() as u64).expect("a range");
        storage.apply_commit(range, data).expect("held");
    }
    storage
}

/// The state after the kernel's header, with `params` on the port and
/// `received` in its receive buffer.
fn after_header(params: VirtualUartPortParams, received: &[u8]) -> UartState {
    let mut uart = UartState::default();
    uart.apply(UartChange::RequestBytesSent(
        PendingRequest::empty(PORT).with(&KERNEL_HEADER),
    ));
    uart.apply(UartChange::PortParamsSet(params));
    uart.apply(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: PORT,
        bytes: received.to_vec(),
    }));
    uart
}

/// The port's values with the receive trigger `rx_trigger` and the mask
/// `mask`, and the transmit trigger that the kernel's open sets: the
/// buffer size less 1 [OFW-4.93 lv2_kernel a:0x80000000000177bc-0x80000000000177ec].
fn params(rx_trigger: u64, mask: u64) -> VirtualUartPortParams {
    VirtualUartPortParams {
        port: PORT,
        tx_trigger: rx_buffer_size() - 1,
        rx_trigger,
        interrupt_mask: mask,
    }
}

fn complete(uart: &UartState) -> HypercallDisposition {
    let r = write_request(REQUEST_ADDRESS, 0x38);
    let storage = storage(&[(REQUEST_ADDRESS, &kernel_request())]);
    write(
        &r,
        &WriteInput {
            firmware: Some("4.93"),
            memory: &MemoryState::default(),
            uart,
            interrupt: &InterruptGroupState::default(),
            storage: &storage,
        },
    )
}

fn queued(written: u64, bytes: Vec<u8>) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [written, 0, 0, 0, 0, 0, 0]);
    answer.push_change(UartChange::RequestBytesSent(PendingRequest::empty(PORT)));
    answer.push_change(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: PORT,
        bytes,
    }));
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

#[test]
fn the_kernel_s_completing_write_writes_0x38_bytes_and_queues_the_reply_under_mask_0() {
    let uart = after_header(params(rx_buffer_size() - 1, 0), &[]);
    assert_eq!(complete(&uart), queued(0x38, kernel_reply()));
}

/// The kernel's read sets the mask 2 after a read of 0 bytes
/// [OFW-4.93 lv2_kernel a:0x8000000000016b9c-0x8000000000016bc4], and its
/// open sets the receive trigger to the buffer size less 1. The test reads
/// the mask of port 10 from the console capture `lv2_vuart_port_mask`.
#[test]
fn the_kernel_s_captured_mask_makes_the_completing_write_raise_the_port_s_shadow_bit() {
    let uart = after_header(params(rx_buffer_size() - 1, kernel_mask()), &[]);
    assert_eq!(complete(&uart), raised(queued(0x38, kernel_reply())));
}

/// The status has the receive bit only when the free bytes are not above
/// the receive trigger [OFW-4.93 lv1 a:0x2F2208-0x2F2210].
#[test]
fn the_captured_mask_raises_the_shadow_bit_only_once_the_free_bytes_reach_the_receive_trigger() {
    let free = rx_buffer_size() - kernel_reply().len() as u64;
    let below = after_header(params(free - 1, kernel_mask()), &[]);
    assert_eq!(complete(&below), queued(0x38, kernel_reply()));
    let at = after_header(params(free, kernel_mask()), &[]);
    assert_eq!(complete(&at), raised(queued(0x38, kernel_reply())));
}

/// `queued` with the raise of port 10's bit, which is bit 63 - 10 of
/// shadow doubleword 0 [OFW-4.93 lv1 a:0x2F2214-0x2F2248]. With no irq
/// configuration, lv1 stores the bit in its own doublewords and gives no
/// event.
fn raised(queued: HypercallDisposition) -> HypercallDisposition {
    let HypercallDisposition::Answered(mut answer) = queued else {
        panic!("an answer to raise: {queued:?}");
    };
    answer.push_change(UartChange::ShadowSet(crate::VirtualUartShadowWrite {
        index: 0,
        value: 1 << 53,
    }));
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_reply_follows_the_bytes_that_the_receive_buffer_holds() {
    let uart = after_header(params(rx_buffer_size() - 1, 0), &[0xEE; 3]);
    let expected = [&[0xEE; 3][..], &kernel_reply()].concat();
    assert_eq!(complete(&uart), queued(0x38, expected));
}

#[test]
fn a_reply_past_the_free_bytes_of_the_receive_buffer_is_a_refusal() {
    let room = rx_buffer_size() as usize - kernel_reply().len();
    let fits = after_header(params(0, 0), &vec![0; room]);
    assert_eq!(
        complete(&fits),
        queued(0x38, [vec![0; room], kernel_reply()].concat())
    );
    let full = after_header(params(0, 0), &vec![0; room + 1]);
    assert_eq!(
        complete(&full),
        refused(
            "a dispatcher manager reply that is more than the free bytes of the port's receive \
             buffer"
        )
    );
}

/// The write of `bytes` from [`REQUEST_ADDRESS`] on `uart`.
fn write_after(uart: &UartState, bytes: &[u8]) -> HypercallDisposition {
    let r = write_request(REQUEST_ADDRESS, bytes.len() as u64);
    let storage = storage(&[(REQUEST_ADDRESS, bytes)]);
    write(
        &r,
        &WriteInput {
            firmware: Some("4.93"),
            memory: &MemoryState::default(),
            uart,
            interrupt: &InterruptGroupState::default(),
            storage: &storage,
        },
    )
}

#[test]
fn a_completing_write_with_no_reply_gets_the_refusal_that_names_its_reason() {
    let request = kernel_request();
    let mut unkept = after_header(params(rx_buffer_size() - 1, 0), &[]);
    unkept.apply(UartChange::RequestBytesSent(
        PendingRequest::empty(PORT)
            .with(&KERNEL_HEADER)
            .with(&request[..9]),
    ));
    assert_eq!(
        write_after(&unkept, &request[9..]),
        refused(reply::UNKEPT_REFUSAL)
    );
    let uart = after_header(params(rx_buffer_size() - 1, 0), &[]);
    let overrun = [&request[..], &[0u8][..]].concat();
    assert_eq!(
        write_after(&uart, &overrun),
        refused(reply::OVERRUN_REFUSAL)
    );
    let mut header = KERNEL_HEADER;
    header[15] = 0x40;
    let mut sized = after_header(params(rx_buffer_size() - 1, 0), &[]);
    sized.apply(UartChange::RequestBytesSent(
        PendingRequest::empty(PORT).with(&header),
    ));
    assert_eq!(
        write_after(&sized, &request),
        HypercallDisposition::Refused(HypercallRefusal::DispatcherRequest {
            unit: UnitId::new(1),
            pc: KERNEL_SC,
            service: 0x19000,
            request: Some(0x19002),
            part: reply::SIZE_PART,
        })
    );
}

#[test]
fn the_hypervisor_queues_the_reply_and_the_kernel_s_reads_take_it_in_order() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let read_at = 0x0010_0000;
    let storage = storage(&[
        (HEADER_ADDRESS, &KERNEL_HEADER),
        (REQUEST_ADDRESS, &kernel_request()),
    ]);
    let dispatch = |hv: &mut Hypervisor, r: &HypercallRequest| match hv.dispatch_with(r, &storage) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal}"),
    };
    let read = |size| request(READ_VIRTUAL_UART, KERNEL_READ_SC, read_at, size);
    assert_eq!(
        dispatch(&mut hv, &write_request(HEADER_ADDRESS, 16)).outputs(),
        Some([16, 0, 0, 0, 0, 0, 0])
    );
    let header_kept = PendingRequest::empty(PORT).with(&KERNEL_HEADER);
    assert_eq!(hv.groups().uart().pending_request(PORT), header_kept);
    let empty = dispatch(&mut hv, &read(16));
    assert_eq!((empty.outputs(), empty.effects.len()), (Some([0; 7]), 0));
    assert_eq!(hv.groups().uart().pending_request(PORT), header_kept);
    assert_eq!(
        dispatch(&mut hv, &write_request(REQUEST_ADDRESS, 0x38)).outputs(),
        Some([0x38, 0, 0, 0, 0, 0, 0])
    );
    assert_eq!(
        hv.groups().uart().pending_request(PORT),
        PendingRequest::empty(PORT)
    );
    assert_eq!(hv.groups().uart().received(PORT), kernel_reply());
    let mut taken = Vec::new();
    for size in [16, 0x38] {
        let answer = dispatch(&mut hv, &read(size));
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
