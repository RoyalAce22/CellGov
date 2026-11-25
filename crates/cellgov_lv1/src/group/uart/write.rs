//! Hypercall 163, `write_virtual_uart`: the call writes bytes to the
//! receive buffer of the peer of a port of the partition's virtual UART.
//!
//! lv1's answer to a call that passes its checks depends on the free bytes
//! in the peer's buffer. The service that holds the peer replies to a
//! complete request; see [`cellgov_ps3_abi::lv1::uart`]. CellGov answers
//! these calls:
//! - a call that fails a check;
//! - a call that writes no byte;
//! - a write to the dispatcher manager's port whose bytes complete no
//!   request.
//!
//! The answer to a write of one or more bytes carries the bytes of the
//! request that is not complete, and the hypervisor keeps them.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::WRITE_VIRTUAL_UART;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::uart::{
    game_os_service_port, game_os_virtual_uart_port, virtual_uart_write_handler,
    GameOsVirtualUartPort, VirtualUartService, GAME_OS_FIRST_DISPATCHER_REQUEST,
    GAME_OS_FIRST_DISPATCHER_SERVICE, VIRTUAL_UART_PORT_MAX, VIRTUAL_UART_WRITE_ADDRESS_MAX,
    WRITE_VIRTUAL_UART_NO_PORT_R3, WRITE_VIRTUAL_UART_NO_REGION_R3, WRITE_VIRTUAL_UART_REGION_R3,
};

use super::buffer::{self, BufferCheck, BufferRegion};
use super::request::PendingRequest;
use super::status;
use super::{UartChange, UartState};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::memory::region::MemoryState;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: WRITE_VIRTUAL_UART,
    name: "write_virtual_uart",
    cost: None,
    handler: |request, input| {
        write(
            request,
            &WriteInput {
                firmware: input.firmware,
                memory: input.groups.memory().regions(),
                uart: input.groups.uart(),
                storage: input.storage,
            },
        )
    },
}];

/// What the handler reads besides the request.
pub(crate) struct WriteInput<'a> {
    /// The firmware version the hypervisor answers for.
    pub(crate) firmware: Option<&'a str>,
    /// The partition's memory regions and their storage.
    pub(crate) memory: &'a MemoryState,
    /// The virtual UART group's state.
    pub(crate) uart: &'a UartState,
    /// The caller's real address space, which holds the bytes to send.
    pub(crate) storage: &'a GuestMemory,
}

/// The refusal of a write that completes a request whose service no source
/// names.
const REQUEST_REFUSAL: &str = "the reply to a complete request on the dispatcher manager's port, \
                               which the dispatcher manager relays from a service or gives from \
                               its own state";

/// The refusal of a write that completes the 4.93 kernel's first request.
const FIRST_REQUEST_REFUSAL: &str = "the reply to request 0x19002 of service 0x19000 on the \
                                     dispatcher manager's port, which the dispatcher manager \
                                     relays from a service or gives from its own state";

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

/// lv1's r3 = 0, with `written` bytes in r4 and r5 to r10 cleared.
fn written_answer(written: u64) -> HypercallAnswer {
    HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [written])
}

/// Hypercall 163: writes the r5 bytes at r4 to the peer of port r3.
///
/// A call that fails one of lv1's checks gets lv1's own r3, with r4 to
/// r10 cleared. The checks, in lv1's order:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`]:
///   [`WRITE_VIRTUAL_UART_NO_PORT_R3`];
/// - r4 is above [`VIRTUAL_UART_WRITE_ADDRESS_MAX`]:
///   [`WRITE_VIRTUAL_UART_NO_REGION_R3`];
/// - no region holds r4: [`WRITE_VIRTUAL_UART_NO_REGION_R3`];
/// - the region has fewer than r5 bytes from r4 to its end:
///   [`WRITE_VIRTUAL_UART_REGION_R3`].
///
/// lv1 checks that it holds the port before it looks up the region.
/// CellGov knows only the ports in the capture, so it makes the region
/// checks only for those ports; see [`buffer::check`].
///
/// Each service port has a peer with a reference: the capture gives it a
/// transmit buffer size other than 0. So it passes lv1's peer check. A call
/// with r5 = 0 that passes the checks on a service port gets r3 = 0 and
/// r4 = 0. A write of one or more bytes to the dispatcher manager's port
/// gets the answer that [`dispatcher_write`] gives.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the port r3 at `firmware`;
/// - r4 is in a region that CellGov does not hold;
/// - r4 is in the real mode region, and no source gives the real mode
///   size for `firmware`;
/// - the call passes the checks for a port that is not a service port,
///   because no source gives the port's peer;
/// - the call writes one or more bytes to the system manager's port,
///   because CellGov does not decode the system manager's requests.
pub(crate) fn write(request: &HypercallRequest, input: &WriteInput<'_>) -> HypercallDisposition {
    let Some(firmware) = input
        .firmware
        .filter(|fw| virtual_uart_write_handler(fw).is_some())
    else {
        return refuse(
            request,
            "a virtual UART write at a firmware whose lv1 handler is not read",
        );
    };
    let (port, address, size) = (request.r(3), request.r(4), request.r(5));
    if port > VIRTUAL_UART_PORT_MAX {
        return lv1_answer(WRITE_VIRTUAL_UART_NO_PORT_R3);
    }
    if address > VIRTUAL_UART_WRITE_ADDRESS_MAX {
        return lv1_answer(WRITE_VIRTUAL_UART_NO_REGION_R3);
    }
    let Some(row) = game_os_virtual_uart_port(firmware, port) else {
        return refuse(
            request,
            "a virtual UART port that no source shows lv1 holds or lacks",
        );
    };
    let region = match buffer::check(firmware, input.memory.regions(), address, size) {
        BufferCheck::Held(region) => region,
        BufferCheck::NoRegion => return lv1_answer(WRITE_VIRTUAL_UART_NO_REGION_R3),
        BufferCheck::TooFewBytes => return lv1_answer(WRITE_VIRTUAL_UART_REGION_R3),
        BufferCheck::RealModeSizeUnknown => {
            return refuse(
                request,
                "a virtual UART write buffer at a firmware whose real mode size no source gives",
            );
        }
        BufferCheck::RegionNotHeld => {
            return refuse(
                request,
                "a virtual UART write buffer in a memory region that CellGov does not hold",
            );
        }
    };
    let Some(service) = game_os_service_port(firmware, port) else {
        return refuse(
            request,
            "a virtual UART write to a port whose peer no source gives",
        );
    };
    if size == 0 {
        return HypercallDisposition::Answered(written_answer(0));
    }
    match service.service {
        VirtualUartService::SystemManager => refuse(
            request,
            "a virtual UART write to the system manager's port, whose requests CellGov does not \
             decode",
        ),
        VirtualUartService::DispatcherManager => {
            let bytes = Buffer {
                address,
                size,
                region,
            };
            dispatcher_write(request, input, firmware, row, &bytes)
        }
    }
}

/// The bytes of a write that pass lv1's checks.
struct Buffer {
    /// The logical partition address of the first byte.
    address: u64,
    /// The count of bytes, which is not 0.
    size: u64,
    /// The memory region that holds the bytes.
    region: BufferRegion,
}

/// The answer to a write of `buffer` to the dispatcher manager's port
/// `row`.
///
/// The answer is r3 = 0 with `buffer.size` bytes written in r4. It
/// carries the bytes of the request that is not complete. The call is a
/// refusal in these cases, in this order:
/// - The bytes can be more than the free bytes of the peer's receive
///   buffer. The dispatcher manager reads the bytes at a time that no image
///   read gives. So the peer's buffer holds at most the bytes of the
///   request that is not complete, and lv1 writes all the bytes when that
///   count and `buffer.size` fit in the buffer.
/// - CellGov's storage does not hold the bytes.
/// - The port's interrupt mask can raise the port's interrupt when lv1
///   computes the status after the write; see [`status::mask_refusal`].
///   CellGov delivers no such interrupt.
/// - The bytes complete a request. The dispatcher manager then replies, and
///   no image read gives the reply. The refusal names the request when it
///   is the 4.93 kernel's first request.
fn dispatcher_write(
    request: &HypercallRequest,
    input: &WriteInput<'_>,
    firmware: &str,
    row: &GameOsVirtualUartPort,
    buffer: &Buffer,
) -> HypercallDisposition {
    let pending = input.uart.pending_request(row.port);
    let fits = pending
        .sent
        .checked_add(buffer.size)
        .is_some_and(|count| count <= row.tx_buffer_size);
    if !fits {
        return refuse(
            request,
            "a virtual UART write whose bytes can be more than the free bytes of the peer's \
             receive buffer",
        );
    }
    let Some(bytes) = sent_bytes(input, buffer) else {
        return refuse(
            request,
            "a virtual UART write buffer whose bytes CellGov's storage does not hold",
        );
    };
    if let Some(what) = status::mask_refusal(firmware, &input.uart.port_params(row.port)) {
        return refuse(request, what);
    }
    let next = pending.with(&bytes);
    if next.is_complete() {
        return refuse(request, request_refusal(&next));
    }
    let mut answer = written_answer(buffer.size);
    answer.push_change(UartChange::RequestBytesSent(next));
    HypercallDisposition::Answered(answer)
}

/// The bytes of `buffer`, read from the storage of its region, or `None`
/// when the storage does not hold them.
///
/// The real mode region lies at real address 0 of the storage. Each
/// allocated region with storage lies at the real address that the memory
/// group gives it.
fn sent_bytes(input: &WriteInput<'_>, buffer: &Buffer) -> Option<Vec<u8>> {
    let real = match buffer.region {
        BufferRegion::RealMode => buffer.address,
        BufferRegion::Allocated(index) => {
            let region = input.memory.regions().get(index)?;
            let storage = input.memory.storage_real(index as u64)?;
            storage.checked_add(buffer.address - region.lpar_address)?
        }
    };
    let range = ByteRange::new(GuestAddr::new(real), buffer.size)?;
    input.storage.read(range).map(|bytes| bytes.into_owned())
}

/// The refusal for the complete request `request`.
fn request_refusal(request: &PendingRequest) -> &'static str {
    let first = request.service() == Some(GAME_OS_FIRST_DISPATCHER_SERVICE)
        && request.first_doubleword() == Some(GAME_OS_FIRST_DISPATCHER_REQUEST);
    if first {
        FIRST_REQUEST_REFUSAL
    } else {
        REQUEST_REFUSAL
    }
}

#[cfg(test)]
#[path = "tests/write_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/write_dispatcher_tests.rs"]
mod dispatcher_tests;
