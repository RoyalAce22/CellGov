use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::WRITE_VIRTUAL_UART;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's read
/// [OFW-4.93 lv2_kernel a:0x8000000000016b7c].
const KERNEL_SC: u64 = 0x8000_0000_0001_6b7c;

/// The ports whose peer the 4.93 system manager and dispatcher manager
/// hold [OFW-4.93 lv1 a:0x44CF04-0x44CF0C] [OFW-4.93 lv1 a:0x5B7E7C].
const SERVICE_PORTS: [u64; 2] = [2, 10];

/// The bytes of a message header, which the kernel's system manager loop
/// reads first [OFW-4.93 lv2_kernel a:0x8000000000264bb0-0x8000000000264bc0].
const HEADER_BYTES: u64 = 16;

/// A buffer address in the real mode region, away from its end.
const BUFFER: u64 = 0x0036_0070;

/// A region that `allocate_memory` created, as the 4.93 kernel's first
/// boot allocation creates it.
const ALLOCATED: MemoryRegion = MemoryRegion {
    lpar_address: 0x0000_6C00_1000_0000,
    size: 0x790_0000,
    page_size_exp: 20,
    unit_id: 0,
    counter: 0,
};

/// A thread 0 call of hypercall 162 at the kernel's `sc`.
fn request(port: u64, address: u64, size: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[port, address, size]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: READ_VIRTUAL_UART,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_548),
    }
}

/// lv1's r3 = 0, with 0 bytes read in r4 and r5 to r10 cleared. lv1's read
/// function reads 0 bytes from an empty buffer with a live peer
/// [OFW-4.93 lv1 a:0x2F09F8-0x2F0A64], and the handler stores the sum in r4
/// [OFW-4.93 lv1 a:0x2F452C-0x2F4538].
fn read_0_bytes() -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [0; 7]))
}

#[test]
fn the_kernel_s_header_read_of_each_service_port_reads_0_bytes() {
    for port in SERVICE_PORTS {
        let r = request(port, BUFFER, HEADER_BYTES);
        assert_eq!(read(&r, Some("4.93"), &[]), read_0_bytes(), "{port}");
    }
}

#[test]
fn a_service_port_read_that_passes_the_checks_reads_0_bytes_for_each_buffer_and_size() {
    let base = ALLOCATED.lpar_address;
    let rm_size = crate::group::memory::region::real_mode_size("4.93").expect("4.93 rm_size");
    let cases = [
        (BUFFER, 0),
        (BUFFER, HEADER_BYTES),
        (rm_size - 16, 16),
        (base, HEADER_BYTES),
        (base + ALLOCATED.size - 16, 16),
    ];
    for port in SERVICE_PORTS {
        for (address, size) in cases {
            let r = request(port, address, size);
            assert_eq!(
                read(&r, Some("4.93"), &[ALLOCATED]),
                read_0_bytes(),
                "{port} {address:#x} {size}"
            );
        }
    }
}

#[test]
fn the_hypervisor_answers_the_kernel_s_service_port_reads_with_0_bytes_and_keeps_its_state() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let storage = GuestMemory::new(0x1000);
    let before = hv.groups().uart().clone();
    for port in SERVICE_PORTS {
        let r = request(port, BUFFER, HEADER_BYTES);
        assert_eq!(hv.dispatch_with(&r, &storage), read_0_bytes(), "{port}");
    }
    assert_eq!(hv.groups().uart(), &before);
}

/// A service writes its port before a shutdown or an external event only
/// to reply to the partition's request; see [`cellgov_ps3_abi::lv1::uart`].
#[test]
fn no_handler_answers_write_virtual_uart_so_the_service_ports_stay_empty() {
    assert!(crate::answered_numbers().contains(&READ_VIRTUAL_UART));
    assert!(!crate::answered_numbers().contains(&WRITE_VIRTUAL_UART));
}
