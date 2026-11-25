use super::*;
use crate::group::memory::region::MemoryRegion;
use crate::group::uart::VirtualUartPortParams;
use crate::{allocated_storage, Hypervisor};
use cellgov_event::UnitId;
use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv1::call::READ_VIRTUAL_UART;
use cellgov_ps3_abi::lv1::uart::VIRTUAL_UART_PORT_CAPTURE;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's write
/// [OFW-4.93 lv2_kernel a:0x8000000000016fd8].
const KERNEL_SC: u64 = 0x8000_0000_0001_6fd8;

/// The `sc` of the 4.93 kernel's read
/// [OFW-4.93 lv2_kernel a:0x8000000000016b7c].
const KERNEL_READ_SC: u64 = 0x8000_0000_0001_6b7c;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// The free bytes of the peer's receive buffer when it holds no byte.
///
/// The value is the transmit buffer size of [`PORT`]'s record in the
/// committed file of the capture `lv2_vuart_port_params`.
fn peer_buffer() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(VIRTUAL_UART_PORT_CAPTURE)
        .join("peek/cech20-cex-493/vuart_ports.tsv");
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
    value(&format!("record{record}_tx")).unwrap_or_else(|| panic!("{name}: no record{record}_tx"))
}

/// The kernel's first boot region in the console capture
/// `lv2_repository_boot_nodes`, which gets no storage.
const FIRST: MemoryRegion = MemoryRegion {
    lpar_address: 0x0000_6C00_1000_0000,
    size: 0x790_0000,
    page_size_exp: 20,
    unit_id: 1,
    counter: 2,
};

/// The kernel's second boot region in the same capture, whose storage
/// starts at the storage base.
const SECOND: MemoryRegion = MemoryRegion {
    lpar_address: 0x0000_5000_0030_0000,
    size: 0xA_0000,
    page_size_exp: 16,
    unit_id: 1,
    counter: 3,
};

/// The address of the kernel's header in CellGov's boot, in [`SECOND`].
const HEADER_ADDRESS: u64 = 0x0000_5000_0038_3b90;

/// The address of the kernel's request bytes in CellGov's boot, in the
/// real mode region.
const REQUEST_ADDRESS: u64 = 0x0049_7940;

/// The header that the 4.93 kernel sends first in CellGov's boot: tag 0,
/// service 0x19000, 0x38 request bytes and 0x38 reply bytes. The kernel's
/// header function builds it [OFW-4.93 lv2_kernel a:0x80000000002121f4];
/// see `cellgov_ps3_abi::lv1::uart`.
const KERNEL_HEADER: [u8; 16] = header(0x19000, 0x38);

/// A header with tag 0 for `service`, with `request_bytes` request bytes
/// and as many reply bytes.
const fn header(service: u32, request_bytes: u32) -> [u8; 16] {
    let s = service.to_be_bytes();
    let n = request_bytes.to_be_bytes();
    [
        0, 0, 0, 0, s[0], s[1], s[2], s[3], n[0], n[1], n[2], n[3], n[0], n[1], n[2], n[3],
    ]
}

/// 0x38 request bytes that start with the request id `id` and the service
/// id `service`, as big-endian doublewords.
fn request_bytes(id: u64, service: u64) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x38];
    bytes[..8].copy_from_slice(&id.to_be_bytes());
    bytes[8..16].copy_from_slice(&service.to_be_bytes());
    bytes
}

fn request(address: u64, size: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[PORT, address, size]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_VIRTUAL_UART,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_776),
    }
}

/// The real mode storage of 8 MB and the storage of the allocated
/// regions, with `bytes` at each real address.
fn storage(bytes: &[(u64, &[u8])]) -> GuestMemory {
    let mut storage = GuestMemory::from_regions(vec![
        Region::new(0, 0x80_0000, "real mode", PageSize::Page64K),
        allocated_storage(),
    ])
    .expect("two regions");
    for &(real, data) in bytes {
        let range = ByteRange::new(GuestAddr::new(real), data.len() as u64).expect("a range");
        storage.apply_commit(range, data).expect("held");
    }
    storage
}

fn boot_regions() -> MemoryState {
    let mut memory = MemoryState::default();
    memory.create(FIRST);
    memory.create(SECOND);
    memory
}

/// The real address of [`HEADER_ADDRESS`] in the boot regions' storage.
fn header_real() -> u64 {
    let base = boot_regions()
        .storage_real(1)
        .expect("the second is stored");
    base + (HEADER_ADDRESS - SECOND.lpar_address)
}

fn with_pending(pending: PendingRequest) -> UartState {
    let mut uart = UartState::default();
    uart.apply(UartChange::RequestBytesSent(pending));
    uart
}

fn write_with(
    r: &HypercallRequest,
    uart: &UartState,
    memory: &MemoryState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    write(
        r,
        &WriteInput {
            firmware: Some("4.93"),
            memory,
            uart,
            storage,
        },
    )
}

fn written(size: u64, next: PendingRequest) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [size, 0, 0, 0, 0, 0, 0]);
    answer.push_change(UartChange::RequestBytesSent(next));
    HypercallDisposition::Answered(answer)
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: WRITE_VIRTUAL_UART,
        unit: r.unit,
        pc: r.pc,
        what,
    })
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

/// The request after the header alone.
fn header_sent(header: [u8; 16]) -> PendingRequest {
    sent(&header)
}

#[test]
fn the_kernel_s_header_write_writes_16_bytes_and_keeps_the_header() {
    let r = request(HEADER_ADDRESS, 16);
    let storage = storage(&[(header_real(), &KERNEL_HEADER)]);
    let mut prefix = [0u8; 24];
    prefix[..16].copy_from_slice(&KERNEL_HEADER);
    let next = PendingRequest {
        port: PORT,
        sent: 16,
        prefix,
    };
    assert_eq!(
        write_with(&r, &UartState::default(), &boot_regions(), &storage),
        written(16, next)
    );
}

#[test]
fn the_kernel_s_request_bytes_complete_its_first_request_and_get_a_refusal_that_names_it() {
    let bytes = request_bytes(0x19002, 0x19000);
    let r = request(REQUEST_ADDRESS, 0x38);
    let storage = storage(&[(REQUEST_ADDRESS, &bytes)]);
    let uart = with_pending(header_sent(KERNEL_HEADER));
    assert_eq!(
        write_with(&r, &uart, &boot_regions(), &storage),
        refused(&r, FIRST_REQUEST_REFUSAL)
    );
}

#[test]
fn a_complete_request_that_is_not_the_kernel_s_first_gets_the_refusal_that_names_no_request() {
    let cases = [
        (header(0x19000, 0x38), request_bytes(0x19003, 0x19000)),
        (header(0x9000, 0x38), request_bytes(0x19002, 0x19000)),
    ];
    for (header, bytes) in cases {
        let r = request(REQUEST_ADDRESS, 0x38);
        let storage = storage(&[(REQUEST_ADDRESS, &bytes)]);
        let uart = with_pending(header_sent(header));
        assert_eq!(
            write_with(&r, &uart, &boot_regions(), &storage),
            refused(&r, REQUEST_REFUSAL),
            "{header:02x?}"
        );
    }
}

#[test]
fn a_header_that_counts_0_request_bytes_completes_with_its_last_byte() {
    let r = request(REQUEST_ADDRESS, 16);
    let storage = storage(&[(REQUEST_ADDRESS, &header(0x19000, 0))]);
    assert_eq!(
        write_with(&r, &UartState::default(), &boot_regions(), &storage),
        refused(&r, REQUEST_REFUSAL)
    );
}

#[test]
fn bytes_of_a_request_that_is_not_complete_are_written_and_added_to_it() {
    let pending = header_sent(KERNEL_HEADER);
    let part = request_bytes(0x19002, 0x19000);
    let r = request(REQUEST_ADDRESS, 0x37);
    let storage = storage(&[(REQUEST_ADDRESS, &part[..0x37])]);
    let uart = with_pending(pending);
    let next = PendingRequest {
        sent: 16 + 0x37,
        ..sent(&[&KERNEL_HEADER[..], &part[..8]].concat())
    };
    assert_eq!(
        write_with(&r, &uart, &boot_regions(), &storage),
        written(0x37, next)
    );
}

#[test]
fn a_header_sent_in_two_writes_is_answered_twice_and_kept_whole() {
    let first = request(REQUEST_ADDRESS, 10);
    let storage_a = storage(&[(REQUEST_ADDRESS, &KERNEL_HEADER[..10])]);
    let after_first = sent(&KERNEL_HEADER[..10]);
    assert_eq!(
        write_with(&first, &UartState::default(), &boot_regions(), &storage_a),
        written(10, after_first)
    );
    let second = request(REQUEST_ADDRESS, 6);
    let storage_b = storage(&[(REQUEST_ADDRESS, &KERNEL_HEADER[10..])]);
    assert_eq!(
        write_with(
            &second,
            &with_pending(after_first),
            &boot_regions(),
            &storage_b
        ),
        written(6, header_sent(KERNEL_HEADER))
    );
}

/// The peer's buffer holds at most the bytes of the request that is not
/// complete, and lv1 copies only the free bytes
/// [OFW-4.93 lv1 a:0x2F0290-0x2F02BC].
#[test]
fn a_write_is_answered_only_while_its_bytes_fit_with_the_request_s_bytes_in_the_peer_s_buffer() {
    let peer_buffer = peer_buffer();
    let pending = PendingRequest {
        sent: 16 + 4000,
        ..header_sent(header(0x19000, 0x2000))
    };
    let uart = with_pending(pending);
    let fits = peer_buffer - (16 + 4000);
    let fill = vec![0u8; fits as usize + 1];
    let storage = storage(&[(REQUEST_ADDRESS, &fill)]);
    let r = request(REQUEST_ADDRESS, fits);
    assert_eq!(
        write_with(&r, &uart, &boot_regions(), &storage),
        written(
            fits,
            PendingRequest {
                sent: peer_buffer,
                ..pending
            }
        )
    );
    let r = request(REQUEST_ADDRESS, fits + 1);
    assert_eq!(
        write_with(&r, &uart, &boot_regions(), &storage),
        refused(
            &r,
            "a virtual UART write whose bytes can be more than the free bytes of the peer's \
             receive buffer"
        )
    );
}

#[test]
fn a_buffer_in_a_region_with_no_storage_is_a_refusal() {
    let r = request(FIRST.lpar_address + 0x100, 16);
    assert_eq!(
        write_with(&r, &UartState::default(), &boot_regions(), &storage(&[])),
        refused(
            &r,
            "a virtual UART write buffer whose bytes CellGov's storage does not hold"
        )
    );
}

/// lv1 calls the port's status function after the write
/// [OFW-4.93 lv1 a:0x2F0548-0x2F059C], and the status has the transmit bit
/// when the peer holds no byte.
#[test]
fn a_kept_mask_with_the_transmit_bit_is_a_refusal_of_the_write() {
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(VirtualUartPortParams {
        port: PORT,
        tx_trigger: 0,
        rx_trigger: 0,
        interrupt_mask: 1,
    }));
    let r = request(HEADER_ADDRESS, 16);
    let storage = storage(&[(header_real(), &KERNEL_HEADER)]);
    assert_eq!(
        write_with(&r, &uart, &boot_regions(), &storage),
        refused(
            &r,
            "the transmit bit 1 of a virtual UART interrupt mask on a port whose peer holds no \
             more bytes than the transmit trigger, with which lv1 raises the port's interrupt"
        )
    );
}

/// A write of 0 bytes never calls the write function
/// [OFW-4.93 lv1 a:0x2F2DE4-0x2F2DF0], and the write function calls the
/// port's listeners only after it copies a byte
/// [OFW-4.93 lv1 a:0x2F053C-0x2F0544].
#[test]
fn a_write_of_0_bytes_is_answered_under_a_kept_mask_with_the_transmit_bit() {
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(VirtualUartPortParams {
        port: PORT,
        tx_trigger: 0,
        rx_trigger: 0,
        interrupt_mask: 1,
    }));
    let r = request(HEADER_ADDRESS, 0);
    assert_eq!(
        write_with(&r, &uart, &boot_regions(), &storage(&[])),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [0; 7]))
    );
}

#[test]
fn the_hypervisor_keeps_the_header_reads_0_bytes_after_it_and_refuses_the_request_bytes() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let header_at = 0x0036_0070;
    let bytes = request_bytes(0x19002, 0x19000);
    let storage = storage(&[(header_at, &KERNEL_HEADER), (REQUEST_ADDRESS, &bytes)]);
    let HypercallDisposition::Answered(answer) =
        hv.dispatch_with(&request(header_at, 16), &storage)
    else {
        panic!("the header write is answered");
    };
    assert_eq!(
        (answer.r3, answer.outputs()),
        (0, Some([16, 0, 0, 0, 0, 0, 0]))
    );
    assert_eq!(
        hv.groups().uart().pending_request(PORT),
        header_sent(KERNEL_HEADER)
    );
    let mut read = request(header_at, 16);
    read.number = READ_VIRTUAL_UART;
    read.pc = KERNEL_READ_SC;
    assert_eq!(
        hv.dispatch_with(&read, &storage),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [0; 7]))
    );
    let r = request(REQUEST_ADDRESS, 0x38);
    assert_eq!(
        hv.dispatch_with(&r, &storage),
        refused(&r, FIRST_REQUEST_REFUSAL)
    );
    assert_eq!(
        hv.groups().uart().pending_request(PORT),
        header_sent(KERNEL_HEADER)
    );
}
