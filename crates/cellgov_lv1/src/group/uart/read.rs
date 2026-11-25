//! Hypercall 162, `read_virtual_uart`: the call reads the bytes in the
//! receive buffer of a port of the partition's virtual UART.
//!
//! lv1's answer to a call that passes its checks depends on the bytes in
//! the buffer. lv1's services write those bytes; see
//! [`cellgov_ps3_abi::lv1::uart`]. CellGov answers these calls:
//! - a call that fails a check;
//! - a read of a service port. The hypervisor holds the bytes of the
//!   port's receive buffer: the replies to the requests that the
//!   partition completed and did not read.
//!
//! The answer to a read of one or more bytes writes them to the caller's
//! buffer, and carries the bytes that the receive buffer still holds.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::READ_VIRTUAL_UART;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::uart::{
    game_os_service_port, game_os_virtual_uart_port, virtual_uart_read_handler,
    READ_VIRTUAL_UART_NO_PORT_R3, READ_VIRTUAL_UART_NO_REGION_R3, READ_VIRTUAL_UART_REGION_R3,
    VIRTUAL_UART_PORT_MAX, VIRTUAL_UART_READ_ADDRESS_MAX,
};

use super::buffer::{self, BufferCheck, BufferRegion};
use super::receive::ReceivedBytes;
use super::shadow::{self, StatusInput};
use super::{UartChange, UartState};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::interrupt::raise::RaiseState;
use crate::group::interrupt::InterruptGroupState;
use crate::group::memory::region::{self as memory, MemoryRegion, MemoryState};
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: READ_VIRTUAL_UART,
    name: "read_virtual_uart",
    cost: None,
    handler: |request, input| {
        read_port(
            request,
            &ReadInput {
                firmware: input.firmware,
                memory: input.groups.memory().regions(),
                uart: input.groups.uart(),
                interrupt: input.groups.interrupt(),
                storage: input.storage,
            },
        )
    },
}];

/// What the handler reads besides the request.
pub(crate) struct ReadInput<'a> {
    /// The firmware version the hypervisor answers for.
    pub(crate) firmware: Option<&'a str>,
    /// The partition's memory regions and their storage.
    pub(crate) memory: &'a MemoryState,
    /// The virtual UART group's state.
    pub(crate) uart: &'a UartState,
    /// The interrupt group's state, which a status step reads.
    pub(crate) interrupt: &'a InterruptGroupState,
    /// The caller's real address space, which holds the caller's buffer.
    pub(crate) storage: &'a GuestMemory,
}

impl ReadInput<'_> {
    /// The state that a status step after the read reads.
    fn status(&self) -> StatusInput<'_> {
        StatusInput {
            uart: self.uart,
            raise: RaiseState {
                bitmaps: self.interrupt.bitmaps(),
                ports: self.interrupt.ports(),
                paused: self.interrupt.paused(),
                storage: self.storage,
            },
        }
    }
}

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

/// A call that passes lv1's checks.
struct Checked<'a> {
    firmware: &'a str,
    port: u64,
    address: u64,
    size: u64,
    region: BufferRegion,
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
/// checks only for those ports; see [`buffer::check`].
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
/// This function answers a call whose port's receive buffer holds no
/// byte. A call that passes the checks for a service port gets r3 = 0 and
/// r4 = 0. lv1's read function reads 0 bytes from an empty buffer with a
/// live peer [OFW-4.93 lv1 a:0x2F09F8-0x2F0A64]. The handler stores the
/// sum, 0, in r4 [OFW-4.93 lv1 a:0x2F452C-0x2F4538].
fn read_with(
    request: &HypercallRequest,
    firmware: Option<&str>,
    regions: &[MemoryRegion],
    real_mode_size: fn(&str) -> Option<u64>,
) -> HypercallDisposition {
    match check(request, firmware, regions, real_mode_size) {
        Ok(checked) => empty_read(request, &checked),
        Err(failed) => failed.answer(request),
    }
}

/// The answer of [`read_with`], with the repository source of the real mode size.
#[cfg(test)]
pub(crate) fn read(
    request: &HypercallRequest,
    firmware: Option<&str>,
    regions: &[MemoryRegion],
) -> HypercallDisposition {
    read_with(request, firmware, regions, memory::real_mode_size)
}

/// Hypercall 162 for the receive buffers that `input.uart` holds.
///
/// A call whose port's receive buffer holds no byte gets the answer of
/// [`read_with`]. A call that passes the checks for a service port whose
/// buffer holds bytes gets the answer of [`received_read`].
pub(crate) fn read_port(request: &HypercallRequest, input: &ReadInput<'_>) -> HypercallDisposition {
    read_port_with(request, input, memory::real_mode_size)
}

/// The answer of [`read_port`], with `real_mode_size` as the source of the real mode size.
fn read_port_with(
    request: &HypercallRequest,
    input: &ReadInput<'_>,
    real_mode_size: fn(&str) -> Option<u64>,
) -> HypercallDisposition {
    let regions = input.memory.regions();
    let held = input.uart.received(request.r(3));
    if held.is_empty() {
        return read_with(request, input.firmware, regions, real_mode_size);
    }
    match check(request, input.firmware, regions, real_mode_size) {
        Ok(checked) => received_read(request, input, &checked, held),
        Err(failed) => failed.answer(request),
    }
}

/// A call that fails one of lv1's checks, or that CellGov refuses before
/// the checks end.
enum Failed {
    /// lv1 returns this r3, with r4 to r10 cleared.
    Lv1(u64),
    /// CellGov does not model these inputs.
    Refused(&'static str),
}

impl Failed {
    fn answer(self, request: &HypercallRequest) -> HypercallDisposition {
        match self {
            Self::Lv1(r3) => lv1_answer(r3),
            Self::Refused(what) => refuse(request, what),
        }
    }
}

/// lv1's checks on `request`, or the failure of a call that fails one.
fn check<'a>(
    request: &HypercallRequest,
    firmware: Option<&'a str>,
    regions: &[MemoryRegion],
    real_mode_size: fn(&str) -> Option<u64>,
) -> Result<Checked<'a>, Failed> {
    let Some(firmware) = firmware.filter(|fw| virtual_uart_read_handler(fw).is_some()) else {
        return Err(Failed::Refused(
            "a virtual UART read at a firmware whose lv1 handler is not read",
        ));
    };
    let (port, address, size) = (request.r(3), request.r(4), request.r(5));
    if port > VIRTUAL_UART_PORT_MAX {
        return Err(Failed::Lv1(READ_VIRTUAL_UART_NO_PORT_R3));
    }
    if address > VIRTUAL_UART_READ_ADDRESS_MAX {
        return Err(Failed::Lv1(READ_VIRTUAL_UART_NO_REGION_R3));
    }
    if game_os_virtual_uart_port(firmware, port).is_none() {
        return Err(Failed::Refused(
            "a virtual UART port that no source shows lv1 holds or lacks",
        ));
    }
    let region = match buffer::check(firmware, regions, address, size, real_mode_size) {
        BufferCheck::Held(region) => region,
        BufferCheck::NoRegion => return Err(Failed::Lv1(READ_VIRTUAL_UART_NO_REGION_R3)),
        BufferCheck::TooFewBytes => return Err(Failed::Lv1(READ_VIRTUAL_UART_REGION_R3)),
        BufferCheck::RealModeSizeUnknown => {
            return Err(Failed::Refused(
                "a virtual UART read buffer at a firmware whose real mode size no source gives",
            ));
        }
        BufferCheck::RegionNotHeld => {
            return Err(Failed::Refused(
                "a virtual UART read buffer in a memory region that CellGov does not hold",
            ));
        }
    };
    Ok(Checked {
        firmware,
        port,
        address,
        size,
        region,
    })
}

/// The answer to a call that passes the checks for a port whose receive
/// buffer holds no byte.
fn empty_read(request: &HypercallRequest, checked: &Checked<'_>) -> HypercallDisposition {
    if game_os_service_port(checked.firmware, checked.port).is_some() {
        return lv1_answer(0);
    }
    refuse(
        request,
        "a virtual UART read of a port whose receive buffer an lv1 service fills",
    )
}

/// The answer to a call that passes the checks for a service port whose
/// receive buffer holds the bytes `held`.
///
/// lv1's read function copies the lesser of the bytes asked for and the
/// bytes in the buffer, in the order that they came
/// [OFW-4.93 lv1 a:0x2F0750-0x2F07F0]. The handler stores the sum in r4
/// [OFW-4.93 lv1 a:0x2F452C-0x2F4538]. A call with r5 = 0 still gives one
/// page with 0 bytes to the read function
/// [OFW-4.93 lv1 a:0x2F447C-0x2F4528]. The read function copies no byte
/// [OFW-4.93 lv1 a:0x2F0798]. It then takes the path of an empty buffer,
/// which returns 0 for a port with a live peer
/// [OFW-4.93 lv1 a:0x2F07F4-0x2F07F8] [OFW-4.93 lv1 a:0x2F09F8-0x2F0A64],
/// and it calls no listener [OFW-4.93 lv1 a:0x2F0830]. So the call gets
/// r3 = 0 and r4 = 0, and the buffer keeps its bytes.
///
/// The answer to a read of one or more bytes writes them to the caller's
/// buffer, and carries the bytes that the receive buffer still holds.
/// lv1 computes the status after the copy of each page
/// [OFW-4.93 lv1 a:0x2F0830-0x2F0940]. Each copy only adds free bytes,
/// so the receive bit can only go from set to clear. The answer carries
/// the step for the bytes left after the read, which can clear the port's
/// shadow bit. The call is a refusal in these cases:
/// - CellGov's storage does not hold the caller's buffer;
/// - the status step after the read refuses; see [`shadow::status_step`].
fn received_read(
    request: &HypercallRequest,
    input: &ReadInput<'_>,
    checked: &Checked<'_>,
    held: &[u8],
) -> HypercallDisposition {
    if game_os_service_port(checked.firmware, checked.port).is_none() {
        return empty_read(request, checked);
    }
    let count = usize::try_from(checked.size).map_or(held.len(), |size| size.min(held.len()));
    if count == 0 {
        return lv1_answer(0);
    }
    let Some(range) = buffer::real_address(input.memory, checked.region, checked.address)
        .and_then(|real| ByteRange::new(GuestAddr::new(real), count as u64))
        .filter(|range| input.storage.holds(range.start().raw(), range.length()))
    else {
        return refuse(
            request,
            "a virtual UART read buffer whose bytes CellGov's storage does not hold",
        );
    };
    let rest = &held[count..];
    let params = input.uart.port_params(checked.port);
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [count as u64]);
    answer.effects.push(Effect::SharedWriteIntent {
        range,
        bytes: WritePayload::new(held[..count].to_vec()),
        source: request.unit,
        source_time: request.tick,
    });
    answer.push_change(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: checked.port,
        bytes: rest.to_vec(),
    }));
    let step = shadow::status_step(
        request,
        checked.firmware,
        &input.status(),
        &params,
        rest.len() as u64,
        &mut answer,
    );
    if let Err(error) = step {
        return error.refusal(request);
    }
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/read_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/read_service_port_tests.rs"]
mod service_port_tests;

#[cfg(test)]
#[path = "tests/read_reply_tests.rs"]
mod reply_tests;

#[cfg(test)]
#[path = "tests/read_real_mode_size_tests.rs"]
mod real_mode_size_tests;
