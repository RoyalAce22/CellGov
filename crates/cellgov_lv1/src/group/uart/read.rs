//! Hypercall 162, `read_virtual_uart`: the call reads the bytes in the
//! receive buffer of a port of the partition's virtual UART.
//!
//! lv1's answer to a call that passes its checks depends on the bytes in
//! the buffer. lv1's services write those bytes; see
//! [`cellgov_ps3_abi::lv1::uart`]. CellGov answers these calls:
//! - a call that fails a check;
//! - a read of a service port, whose receive buffer holds no byte while
//!   CellGov answers no `write_virtual_uart`.
//!
//! The answer reads only the memory regions that the hypervisor holds, and
//! carries no change.

use cellgov_ps3_abi::lv1::call::READ_VIRTUAL_UART;
use cellgov_ps3_abi::lv1::interrupt::region_lookup_base;
use cellgov_ps3_abi::lv1::uart::{
    game_os_service_port, game_os_virtual_uart_port, virtual_uart_read_handler,
    READ_VIRTUAL_UART_NO_PORT_R3, READ_VIRTUAL_UART_NO_REGION_R3, READ_VIRTUAL_UART_REGION_R3,
    VIRTUAL_UART_PORT_MAX, VIRTUAL_UART_READ_ADDRESS_MAX,
};

use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::memory::region::{self as memory, MemoryRegion};
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: READ_VIRTUAL_UART,
    name: "read_virtual_uart",
    cost: None,
    handler: |request, input| {
        read(
            request,
            input.firmware,
            input.groups.memory().regions().regions(),
        )
    },
}];

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 162: reads up to r5 bytes of port r3 into the buffer at r4.
///
/// A call that fails one of lv1's checks gets lv1's own r3, with r4 to
/// r10 cleared. The checks, in lv1's order:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`]:
///   [`READ_VIRTUAL_UART_NO_PORT_R3`];
/// - r4 is above [`VIRTUAL_UART_READ_ADDRESS_MAX`]:
///   [`READ_VIRTUAL_UART_NO_REGION_R3`];
/// - no region holds r4: [`READ_VIRTUAL_UART_NO_REGION_R3`];
/// - the region has fewer than r5 bytes from r4 to its end:
///   [`READ_VIRTUAL_UART_REGION_R3`].
///
/// lv1 checks that it holds the port before it looks up the region.
/// CellGov knows only the ports in the capture, so it makes the region
/// checks only for those ports. It makes them for these regions:
/// - the real mode region, whose base is 0. lv1 checks its class and its
///   page array as it does for `configure_virtual_uart_irq`, whose kernel
///   call passes them on the console.
/// - each region that `allocate_memory` created. Each passes the class
///   check and the page array check; see [`allocated_region_size`].
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the port r3 at `firmware`;
/// - r4 is in a region that CellGov does not hold;
/// - r4 is in the real mode region, and no source gives the real mode
///   size for `firmware`;
/// - the call passes the checks for a port that is not a service port,
///   because no source gives the bytes in that port's receive buffer.
///
/// A call that passes the checks for a service port gets r3 = 0 and
/// r4 = 0:
/// - The port's service writes nothing to the port before the partition's
///   first request; see [`cellgov_ps3_abi::lv1::uart`].
/// - The partition sends a request with `write_virtual_uart`, which CellGov
///   does not answer. So the port's receive buffer is empty.
/// - lv1's read function reads 0 bytes from an empty buffer with a live
///   peer [OFW-4.93 lv1 a:0x2F09F8-0x2F0A64]. The handler stores the sum,
///   0, in r4 [OFW-4.93 lv1 a:0x2F452C-0x2F4538].
pub(crate) fn read(
    request: &HypercallRequest,
    firmware: Option<&str>,
    regions: &[MemoryRegion],
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| virtual_uart_read_handler(fw).is_some()) else {
        return refuse(
            request,
            "a virtual UART read at a firmware whose lv1 handler is not read",
        );
    };
    let (port, address, size) = (request.r(3), request.r(4), request.r(5));
    if port > VIRTUAL_UART_PORT_MAX {
        return lv1_answer(READ_VIRTUAL_UART_NO_PORT_R3);
    }
    if address > VIRTUAL_UART_READ_ADDRESS_MAX {
        return lv1_answer(READ_VIRTUAL_UART_NO_REGION_R3);
    }
    if game_os_virtual_uart_port(firmware, port).is_none() {
        return refuse(
            request,
            "a virtual UART port that no source shows lv1 holds or lacks",
        );
    }
    let base = region_lookup_base(address);
    let region_size = if base == 0 {
        let Some(rm_size) = memory::real_mode_size(firmware) else {
            return refuse(
                request,
                "a virtual UART read buffer at a firmware whose real mode size no source gives",
            );
        };
        rm_size
    } else {
        let Some(region_size) = allocated_region_size(regions, base) else {
            return refuse(
                request,
                "a virtual UART read buffer in a memory region that CellGov does not hold",
            );
        };
        region_size
    };
    // lv1's lookup finds no region when the base plus the size is not
    // above r4 [OFW-4.93 lv1 a:0x2CE798-0x2CE7A8].
    let offset = address - base;
    if offset >= region_size {
        return lv1_answer(READ_VIRTUAL_UART_NO_REGION_R3);
    }
    if region_size - offset < size {
        return lv1_answer(READ_VIRTUAL_UART_REGION_R3);
    }
    if game_os_service_port(firmware, port).is_some() {
        return lv1_answer(0);
    }
    refuse(
        request,
        "a virtual UART read of a port whose receive buffer an lv1 service fills",
    )
}

/// The size of the region that `allocate_memory` created at the base
/// `base`, or `None` when the hypervisor holds no such region.
///
/// Such a region passes the class check and the page array check of
/// `read_virtual_uart`:
/// - The constructor that the worker calls [OFW-4.93 lv1 a:0x2DF244]
///   stores the word of the TOC slot at r2 - 8096 at +72
///   [OFW-4.93 lv1 a:0x2DEA48] [OFW-4.93 lv1 a:0x2DEAAC]
///   [OFW-4.93 lv1 a:0x2CF474] [OFW-4.93 lv1 a:0x2CF4C4]. That word
///   [OFW-4.93 lv1 a:0x358098] is the word of the slot at r2 - 5872 that
///   the handler checks [OFW-4.93 lv1 a:0x358948].
/// - The constructor stores the region's page object at +176
///   [OFW-4.93 lv1 a:0x2DEAE0-0x2DEAE8].
fn allocated_region_size(regions: &[MemoryRegion], base: u64) -> Option<u64> {
    regions
        .iter()
        .find(|region| region.lpar_address == base)
        .map(|region| region.size)
}

#[cfg(test)]
#[path = "tests/read_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/read_service_port_tests.rs"]
mod service_port_tests;
