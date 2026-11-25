use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's read
/// [OFW-4.93 lv2_kernel a:0x8000000000016b7c].
const KERNEL_SC: u64 = 0x8000_0000_0001_6b7c;

/// The ports that the 4.93 kernel opened on the console, from the capture
/// `lv2_vuart_port_params`.
const KERNEL_PORTS: [u64; 3] = [0, 2, 10];

/// The system manager port, whose receive loop makes the kernel's first
/// read [OFW-4.93 lv2_kernel a:0x8000000000264388].
const SYSTEM_MANAGER_PORT: u64 = 2;

/// The bytes of the system manager's message header, which the receive
/// loop reads first [OFW-4.93 lv2_kernel a:0x8000000000264bb0-0x8000000000264bc0].
const HEADER_BYTES: u64 = 16;

/// The 4.93 GameOS partition's real mode size, `bi.pu.1.rm_size`.
const RM_SIZE: u64 = 0x0800_0000;

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

const READ_WHAT: &str = "a virtual UART read of a port whose receive buffer an lv1 service fills";

const NO_REGION_WHAT: &str =
    "a virtual UART read buffer in a memory region that CellGov does not hold";

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

fn at_4_93(r: &HypercallRequest) -> HypercallDisposition {
    read(r, Some("4.93"), &[])
}

fn with_allocated(r: &HypercallRequest) -> HypercallDisposition {
    read(r, Some("4.93"), &[ALLOCATED])
}

fn lv1_r3(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: READ_VIRTUAL_UART,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn the_kernel_s_header_read_of_the_system_manager_port_is_a_refusal() {
    let r = request(SYSTEM_MANAGER_PORT, BUFFER, HEADER_BYTES);
    assert_eq!(at_4_93(&r), refused(&r, READ_WHAT));
}

/// A size of 0 passes lv1's checks, and lv1 still calls the read function
/// [OFW-4.93 lv1 a:0x2F447C-0x2F44F4].
#[test]
fn a_read_that_passes_the_checks_is_a_refusal_for_each_kernel_port_and_size() {
    for port in KERNEL_PORTS {
        for (address, size) in [(BUFFER, HEADER_BYTES), (BUFFER, 0), (RM_SIZE - 16, 16)] {
            let r = request(port, address, size);
            assert_eq!(
                at_4_93(&r),
                refused(&r, READ_WHAT),
                "{port} {address:#x} {size}"
            );
        }
    }
}

/// lv1 refuses r3 above 255 [OFW-4.93 lv1 a:0x2F433C-0x2F4340].
#[test]
fn a_port_above_255_gets_lv1_s_no_port_r3() {
    for port in [VIRTUAL_UART_PORT_MAX + 1, 0x1_0000_0002, u64::MAX] {
        assert_eq!(
            at_4_93(&request(port, BUFFER, HEADER_BYTES)),
            lv1_r3(READ_VIRTUAL_UART_NO_PORT_R3),
            "{port:#x}"
        );
    }
}

/// lv1 checks the address bound before it reads the port's slot
/// [OFW-4.93 lv1 a:0x2F4344-0x2F4354], so a port that no source gives
/// still gets lv1's r3.
#[test]
fn an_address_above_the_bound_gets_lv1_s_no_region_r3_before_the_port_lookup() {
    for port in [SYSTEM_MANAGER_PORT, 1] {
        for address in [VIRTUAL_UART_READ_ADDRESS_MAX + 1, u64::MAX] {
            assert_eq!(
                at_4_93(&request(port, address, HEADER_BYTES)),
                lv1_r3(READ_VIRTUAL_UART_NO_REGION_R3),
                "{port} {address:#x}"
            );
        }
    }
}

/// lv1 refuses only an r4 above the bound [OFW-4.93 lv1 a:0x2F434C-0x2F4350],
/// so an r4 at the bound reaches the port's slot.
#[test]
fn an_address_at_the_bound_passes_the_bound_check_to_the_port_lookup() {
    let r = request(1, VIRTUAL_UART_READ_ADDRESS_MAX, HEADER_BYTES);
    assert_eq!(
        at_4_93(&r),
        refused(
            &r,
            "a virtual UART port that no source shows lv1 holds or lacks"
        )
    );
}

#[test]
fn a_port_that_no_source_gives_is_a_refusal() {
    for port in [1, 3, 11, VIRTUAL_UART_PORT_MAX] {
        let r = request(port, BUFFER, HEADER_BYTES);
        assert_eq!(
            at_4_93(&r),
            refused(
                &r,
                "a virtual UART port that no source shows lv1 holds or lacks"
            ),
            "{port}"
        );
    }
}

/// The real mode region holds the addresses below `rm_size`, and lv1's
/// lookup finds no region for the others [OFW-4.93 lv1 a:0x2F4440-0x2F4444].
#[test]
fn a_buffer_in_no_region_gets_lv1_s_no_region_r3() {
    for address in [RM_SIZE, RM_SIZE + 0x1000] {
        assert_eq!(
            at_4_93(&request(SYSTEM_MANAGER_PORT, address, HEADER_BYTES)),
            lv1_r3(READ_VIRTUAL_UART_NO_REGION_R3),
            "{address:#x}"
        );
    }
}

/// lv1 refuses a region with fewer than r5 bytes from r4 to its end
/// [OFW-4.93 lv1 a:0x2F445C-0x2F446C].
#[test]
fn a_buffer_past_the_real_mode_region_s_end_gets_lv1_s_region_r3() {
    for (address, size) in [(RM_SIZE - 15, 16), (RM_SIZE - 1, 2), (0, RM_SIZE + 1)] {
        assert_eq!(
            at_4_93(&request(SYSTEM_MANAGER_PORT, address, size)),
            lv1_r3(READ_VIRTUAL_UART_REGION_R3),
            "{address:#x} {size}"
        );
    }
}

#[test]
fn a_buffer_in_a_region_that_cellgov_does_not_hold_is_a_refusal() {
    let held_elsewhere = (28 << 42) | 0x1000_0000;
    for (address, regions) in [
        (held_elsewhere, &[ALLOCATED][..]),
        (ALLOCATED.lpar_address + 0x1_0000, &[][..]),
    ] {
        let r = request(SYSTEM_MANAGER_PORT, address, HEADER_BYTES);
        assert_eq!(
            read(&r, Some("4.93"), regions),
            refused(&r, NO_REGION_WHAT),
            "{address:#x}"
        );
    }
}

#[test]
fn a_read_into_an_allocated_region_that_passes_the_checks_is_a_refusal() {
    let base = ALLOCATED.lpar_address;
    for (address, size) in [
        (base, HEADER_BYTES),
        (base + 0x1_0070, HEADER_BYTES),
        (base + ALLOCATED.size - 16, 16),
    ] {
        let r = request(SYSTEM_MANAGER_PORT, address, size);
        assert_eq!(
            with_allocated(&r),
            refused(&r, READ_WHAT),
            "{address:#x} {size}"
        );
    }
}

/// lv1's lookup finds no region at or past the region's end
/// [OFW-4.93 lv1 a:0x2CE798-0x2CE7A8], and refuses a region with fewer
/// than r5 bytes from r4 to its end [OFW-4.93 lv1 a:0x2F445C-0x2F446C].
#[test]
fn a_buffer_at_or_past_an_allocated_region_s_end_gets_lv1_s_region_r3s() {
    let end = ALLOCATED.lpar_address + ALLOCATED.size;
    for (address, size, r3) in [
        (end, HEADER_BYTES, READ_VIRTUAL_UART_NO_REGION_R3),
        (end + 0x1000, HEADER_BYTES, READ_VIRTUAL_UART_NO_REGION_R3),
        (end - 15, 16, READ_VIRTUAL_UART_REGION_R3),
        (end - 1, 2, READ_VIRTUAL_UART_REGION_R3),
    ] {
        assert_eq!(
            with_allocated(&request(SYSTEM_MANAGER_PORT, address, size)),
            lv1_r3(r3),
            "{address:#x} {size}"
        );
    }
}

#[test]
fn a_read_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let r = request(SYSTEM_MANAGER_PORT, BUFFER, HEADER_BYTES);
    let what = "a virtual UART read at a firmware whose lv1 handler is not read";
    for firmware in [None, Some("4.92")] {
        assert_eq!(read(&r, firmware, &[]), refused(&r, what), "{firmware:?}");
    }
}

#[test]
fn the_hypervisor_dispatches_162_to_the_read_and_keeps_its_state() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let storage = GuestMemory::new(0x1000);
    let before = hv.groups().uart().clone();
    let checked = request(VIRTUAL_UART_PORT_MAX + 1, BUFFER, HEADER_BYTES);
    assert_eq!(
        hv.dispatch_with(&checked, &storage),
        lv1_r3(READ_VIRTUAL_UART_NO_PORT_R3)
    );
    let kernel = request(SYSTEM_MANAGER_PORT, BUFFER, HEADER_BYTES);
    assert_eq!(
        hv.dispatch_with(&kernel, &storage),
        refused(&kernel, READ_WHAT)
    );
    assert_eq!(hv.groups().uart(), &before);
}
