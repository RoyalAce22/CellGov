use super::*;
use crate::group::interrupt::InterruptGroupState;
use crate::group::memory::region::MemoryRegion;
use cellgov_event::UnitId;
use cellgov_mem::{PageSize, Region};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's write
/// [OFW-4.93 lv2_kernel a:0x8000000000016fd8].
const KERNEL_SC: u64 = 0x8000_0000_0001_6fd8;

/// The port that the 4.93 kernel opened on the console, from the capture
/// `lv2_vuart_port_params`, whose peer no image read shows.
const NON_SERVICE_PORT: u64 = 0;

/// The system manager's port [OFW-4.93 lv1 a:0x44CF04-0x44CF0C].
const SYSTEM_MANAGER_PORT: u64 = 2;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const DISPATCHER_PORT: u64 = 10;

/// The 4.93 GameOS partition's real mode size, `bi.pu.1.rm_size`.
const RM_SIZE: u64 = 0x0800_0000;

/// A buffer address in the real mode region, away from its end.
const BUFFER: u64 = 0x0049_7940;

/// The kernel's first boot region in the console capture
/// `lv2_repository_boot_nodes`. The guest memory of these tests does not
/// hold the region's storage.
const FIRST: MemoryRegion = MemoryRegion {
    lpar_address: 0x0000_6C00_1000_0000,
    size: 0x790_0000,
    page_size_exp: 20,
    unit_id: 1,
    counter: 2,
};

const PORT_WHAT: &str = "a virtual UART port that no source shows lv1 holds or lacks";

/// A thread 0 call of hypercall 163 at the kernel's `sc`.
fn request(port: u64, address: u64, size: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[port, address, size]);
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

/// A real mode storage of 8 MB, which holds [`BUFFER`].
fn storage() -> GuestMemory {
    GuestMemory::from_regions(vec![Region::new(
        0,
        0x80_0000,
        "real mode",
        PageSize::Page64K,
    )])
    .expect("one region")
}

fn with_first_region() -> MemoryState {
    let mut memory = MemoryState::default();
    memory.create(FIRST);
    memory
}

fn write_at(
    firmware: Option<&str>,
    r: &HypercallRequest,
    memory: &MemoryState,
) -> HypercallDisposition {
    write(
        r,
        &WriteInput {
            firmware,
            memory,
            uart: &UartState::default(),
            interrupt: &InterruptGroupState::default(),
            storage: &storage(),
        },
    )
}

fn at_4_93(r: &HypercallRequest) -> HypercallDisposition {
    write_at(Some("4.93"), r, &with_first_region())
}

fn lv1_r3(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: WRITE_VIRTUAL_UART,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn a_firmware_whose_lv1_handler_is_not_read_refuses_each_call() {
    for firmware in [None, Some("4.92"), Some("3.70")] {
        let r = request(DISPATCHER_PORT, BUFFER, 16);
        assert_eq!(
            write_at(firmware, &r, &with_first_region()),
            refused(
                &r,
                "a virtual UART write at a firmware whose lv1 handler is not read"
            ),
            "{firmware:?}"
        );
    }
}

/// The port bound [OFW-4.93 lv1 a:0x2F2B90-0x2F2B94] comes before the
/// address bound [OFW-4.93 lv1 a:0x2F2B98-0x2F2BA4], and each holds for a
/// port that no source gives.
#[test]
fn a_port_above_255_gets_lv1_s_port_r3_before_the_address_bound() {
    for port in [256, 0x1_0000, u64::MAX] {
        let r = request(port, u64::MAX, 16);
        assert_eq!(at_4_93(&r), lv1_r3(WRITE_VIRTUAL_UART_NO_PORT_R3), "{port}");
    }
}

/// lv1 refuses only an r4 above 2^62 - 1, unsigned
/// [OFW-4.93 lv1 a:0x2F2B98-0x2F2BA4], so an r4 at the bound reaches the
/// port lookup.
#[test]
fn only_an_address_above_2_to_the_62_less_1_gets_lv1_s_region_r3_for_any_port() {
    for port in [0, 1, 10, 255] {
        let r = request(port, VIRTUAL_UART_WRITE_ADDRESS_MAX + 1, 16);
        assert_eq!(
            at_4_93(&r),
            lv1_r3(WRITE_VIRTUAL_UART_NO_REGION_R3),
            "{port}"
        );
    }
    let r = request(1, VIRTUAL_UART_WRITE_ADDRESS_MAX, 16);
    assert_eq!(at_4_93(&r), refused(&r, PORT_WHAT));
}

#[test]
fn a_port_that_no_source_gives_is_a_refusal() {
    for port in [1, 3, 9, 11, 255] {
        let r = request(port, BUFFER, 16);
        assert_eq!(at_4_93(&r), refused(&r, PORT_WHAT), "{port}");
    }
}

#[test]
fn a_buffer_past_the_end_of_its_region_gets_lv1_s_region_r3s() {
    let end = FIRST.lpar_address + FIRST.size;
    let cases = [
        (RM_SIZE, 1, WRITE_VIRTUAL_UART_NO_REGION_R3),
        (RM_SIZE - 8, 16, WRITE_VIRTUAL_UART_REGION_R3),
        (end, 1, WRITE_VIRTUAL_UART_NO_REGION_R3),
        (end - 8, 16, WRITE_VIRTUAL_UART_REGION_R3),
    ];
    for (address, size, r3) in cases {
        let r = request(DISPATCHER_PORT, address, size);
        assert_eq!(at_4_93(&r), lv1_r3(r3), "{address:#x} {size}");
    }
}

/// lv1 makes the region checks [OFW-4.93 lv1 a:0x2F2C6C-0x2F2CCC] before
/// the peer check [OFW-4.93 lv1 a:0x2F2CF8-0x2F2D3C].
#[test]
fn a_buffer_past_the_end_of_its_region_gets_lv1_s_region_r3s_before_the_peer_check() {
    let cases = [
        (RM_SIZE, 0, WRITE_VIRTUAL_UART_NO_REGION_R3),
        (RM_SIZE - 8, 16, WRITE_VIRTUAL_UART_REGION_R3),
    ];
    for port in [NON_SERVICE_PORT, SYSTEM_MANAGER_PORT] {
        for (address, size, r3) in cases {
            let r = request(port, address, size);
            assert_eq!(at_4_93(&r), lv1_r3(r3), "{port} {address:#x} {size}");
        }
    }
}

/// lv1 reads the port's endpoint [OFW-4.93 lv1 a:0x2F2BA8-0x2F2C68] before
/// it looks up the region [OFW-4.93 lv1 a:0x2F2C6C-0x2F2C98].
#[test]
fn a_port_that_no_source_gives_is_a_refusal_before_the_region_checks() {
    for (address, size) in [(RM_SIZE, 1), (RM_SIZE - 8, 16)] {
        let r = request(1, address, size);
        assert_eq!(at_4_93(&r), refused(&r, PORT_WHAT), "{address:#x} {size}");
    }
}

#[test]
fn a_buffer_in_a_region_that_cellgov_does_not_hold_is_a_refusal() {
    let r = request(DISPATCHER_PORT, 0x0000_5000_0030_0000, 16);
    for memory in [MemoryState::default(), with_first_region()] {
        assert_eq!(
            write_at(Some("4.93"), &r, &memory),
            refused(
                &r,
                "a virtual UART write buffer in a memory region that CellGov does not hold"
            ),
            "{} regions",
            memory.regions().len()
        );
    }
}

#[test]
fn a_write_to_a_port_whose_peer_no_source_gives_is_a_refusal_even_for_0_bytes() {
    for size in [0, 16] {
        let r = request(NON_SERVICE_PORT, BUFFER, size);
        assert_eq!(
            at_4_93(&r),
            refused(
                &r,
                "a virtual UART write to a port whose peer no source gives"
            ),
            "{size}"
        );
    }
}

/// The handler's page loop takes the lesser of the bytes left in the page
/// and r5, and stops at 0 before it calls the write function
/// [OFW-4.93 lv1 a:0x2F2DC0-0x2F2DF0]. It then stores the sum, 0, and
/// returns 0 [OFW-4.93 lv1 a:0x2F2DF4-0x2F2E00].
#[test]
fn a_write_of_0_bytes_to_a_service_port_writes_no_byte_and_carries_no_change() {
    for port in [SYSTEM_MANAGER_PORT, DISPATCHER_PORT] {
        let r = request(port, BUFFER, 0);
        assert_eq!(at_4_93(&r), lv1_r3(0), "{port}");
    }
}

/// A buffer that ends at its region's end passes the check of the r5 bytes
/// from r4 to the end [OFW-4.93 lv1 a:0x2F2CB0-0x2F2CC0]. Each byte of
/// the storage is 0, so a 16-byte write is a header that counts 0 payload
/// bytes.
#[test]
fn a_write_of_bytes_to_the_system_manager_s_port_passes_lv1_s_region_checks() {
    let end = FIRST.lpar_address + FIRST.size;
    let unheld = "a virtual UART write buffer whose bytes CellGov's storage does not hold";
    let mut answer = HypercallAnswer::lv1_status(0, [16, 0, 0, 0, 0, 0, 0]);
    answer.push_change(UartChange::RequestBytesSent(PendingRequest::empty(
        SYSTEM_MANAGER_PORT,
    )));
    let r = |address, size| request(SYSTEM_MANAGER_PORT, address, size);
    let cases = [
        (
            r(BUFFER, 1),
            refused(&r(BUFFER, 1), system_manager::HEADER_SPLIT_REFUSAL),
        ),
        (r(BUFFER, 16), HypercallDisposition::Answered(answer)),
        (
            r(BUFFER, 0x800),
            refused(&r(BUFFER, 0x800), system_manager::OVERRUN_REFUSAL),
        ),
        (r(RM_SIZE - 16, 16), refused(&r(RM_SIZE - 16, 16), unheld)),
        (r(end - 16, 16), refused(&r(end - 16, 16), unheld)),
    ];
    for (call, expected) in cases {
        assert_eq!(at_4_93(&call), expected, "{:#x} {}", call.r(4), call.r(5));
    }
}
