//! Hypercall 166, `configure_virtual_uart_irq`: the call gives the
//! partition's virtual UART an irq outlet and a 32-byte interrupt bitmap.
//!
//! lv1 constructs the outlet with a copy of the event receive port
//! constructor. `connect_irq_plug_ext` connects it as it connects an irq
//! outlet of a logical SPE; see [`cellgov_ps3_abi::lv1::uart`]. So the
//! answer gives the outlet to the interrupt group's irq outlets. The
//! virtual UART group keeps the outlet id and the bitmap address, and
//! hashes them.
//!
//! A console capture gives the outlet id of the GameOS kernel's call by
//! the call's `sc` address. So the handler reads
//! [`HypercallRequest::pc`], although lv1 does not.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::CONFIGURE_VIRTUAL_UART_IRQ;
use cellgov_ps3_abi::lv1::interrupt::region_lookup_base;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::uart::{
    game_os_virtual_uart_outlet, virtual_uart_irq_bitmap_in_small_page, virtual_uart_irq_handler,
    CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3, CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3,
    CONFIGURE_VIRTUAL_UART_IRQ_REGION_R3, VIRTUAL_UART_IRQ_ADDRESS_MAX,
    VIRTUAL_UART_IRQ_BITMAP_BYTES,
};

use super::shadow::UART_SHADOW_DOUBLEWORDS;
use super::{UartChange, UartState};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::interrupt::event_port::{EventReceivePort, EventReceivePortState};
use crate::group::interrupt::InterruptChange;
use crate::group::memory::region as memory;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: CONFIGURE_VIRTUAL_UART_IRQ,
    name: "configure_virtual_uart_irq",
    cost: None,
    handler: |request, input| {
        configure_irq(
            request,
            input.firmware,
            &IrqState {
                uart: input.groups.uart(),
                ports: input.groups.interrupt().ports(),
                storage: input.storage,
            },
        )
    },
}];

/// The interrupt configuration that `configure_virtual_uart_irq` gave the
/// partition's virtual UART.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartIrq {
    /// The id of the virtual UART's irq outlet, from r4 of the answer.
    pub outlet: u64,
    /// The bitmap's logical partition address, from r3 of the call.
    pub bitmap: u64,
}

/// Fields 1 and 2: the outlet id and the bitmap address.
impl LaneValue for VirtualUartIrq {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.outlet);
        lanes.lane(2, 0, self.bitmap);
    }
}

/// The state that the hypervisor holds for a configure step.
pub(crate) struct IrqState<'a> {
    /// The virtual UART's interrupt configuration.
    pub uart: &'a UartState,
    /// The event receive ports and the irq outlets, whose ids the outlet
    /// must not take.
    pub ports: &'a EventReceivePortState,
    /// The caller's real address space, which holds the bitmap.
    pub storage: &'a GuestMemory,
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

/// Hypercall 166: gives the virtual UART an irq outlet, with its id in r4, and the bitmap at r3.
///
/// The answer clears r5 to r10 and writes the virtual UART's shadow to
/// the 32 bytes of the bitmap, as lv1 copies it there. The answer carries
/// the outlet with no connection, and the interrupt configuration. The
/// hypervisor keeps both. For a shadow with a set bit, lv1 gives the
/// outlet to the event function. The new outlet has no plug, so the
/// answer only marks the outlet's event as pending.
///
/// A call that fails one of lv1's checks gets lv1's own r3, with r4 to
/// r10 cleared. See [`CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3`] for the
/// checks and their order. CellGov answers the region checks for the real
/// mode region only, whose base is 0. The kernel's call passes the checks
/// that read the region's own fields on the console, because lv1 gave it
/// an outlet.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is outside the real mode region;
/// - no source gives the real mode size for `firmware`;
/// - the bitmap and its next byte are in two 4 KB pages, because no
///   source gives the region's page size;
/// - no source gives the outlet id for the call's address at `firmware`;
/// - the hypervisor already holds a port or an irq outlet with that id;
/// - no storage holds the bitmap.
pub(crate) fn configure_irq(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &IrqState<'_>,
) -> HypercallDisposition {
    configure_irq_with(request, firmware, state, memory::real_mode_size)
}

/// The answer of [`configure_irq`], with `real_mode_size` as the source of the real mode size.
fn configure_irq_with(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &IrqState<'_>,
    real_mode_size: fn(&str) -> Option<u64>,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| virtual_uart_irq_handler(fw).is_some()) else {
        return refuse(
            request,
            "a virtual UART interrupt call at a firmware whose lv1 handler is not read",
        );
    };
    let address = request.r(3);
    if address > VIRTUAL_UART_IRQ_ADDRESS_MAX {
        return lv1_answer(CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3);
    }
    if region_lookup_base(address) != 0 {
        return refuse(
            request,
            "a virtual UART bitmap outside the partition's real mode region",
        );
    }
    let Some(rm_size) = real_mode_size(firmware) else {
        return refuse(
            request,
            "a virtual UART bitmap at a firmware whose real mode size no source gives",
        );
    };
    if address >= rm_size {
        return lv1_answer(CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3);
    }
    if rm_size - address < VIRTUAL_UART_IRQ_BITMAP_BYTES {
        return lv1_answer(CONFIGURE_VIRTUAL_UART_IRQ_REGION_R3);
    }
    if !virtual_uart_irq_bitmap_in_small_page(address) {
        return refuse(
            request,
            "a virtual UART bitmap across a 4 KB page, whose region page size no source gives",
        );
    }
    if state.uart.irq().is_some() {
        return lv1_answer(CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3);
    }
    let Some(outlet) = game_os_virtual_uart_outlet(firmware, request.pc) else {
        return refuse(
            request,
            "a virtual UART interrupt call whose outlet id no source gives for its address",
        );
    };
    // The outlet takes its id from the object table of the ports and the
    // other outlets; see [`cellgov_ps3_abi::lv1::uart`].
    if state.ports.port(outlet).is_some() || state.ports.outlet(outlet).is_some() {
        return refuse(
            request,
            "a virtual UART interrupt call whose outlet id the hypervisor already holds",
        );
    }
    let Some(range) = ByteRange::new(GuestAddr::new(address), VIRTUAL_UART_IRQ_BITMAP_BYTES)
        .filter(|range| state.storage.holds(range.start().raw(), range.length()))
    else {
        return refuse(
            request,
            "a virtual UART bitmap that CellGov holds no storage for",
        );
    };
    let shadow = state.uart.shadow();
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [outlet]);
    answer.effects.push(Effect::SharedWriteIntent {
        range,
        bytes: WritePayload::new(shadow.iter().flat_map(|d| d.to_be_bytes()).collect()),
        source: request.unit,
        source_time: request.tick,
    });
    answer.push_change(InterruptChange::OutletConstructed(EventReceivePort::new(
        outlet,
    )));
    // lv1 gives the outlet to the event function while the count of set
    // shadow bits is above 0 [OFW-4.93 lv1 a:0x2F16F4-0x2F1708].
    if shadow != [0; UART_SHADOW_DOUBLEWORDS] {
        answer.push_change(InterruptChange::EventPending(outlet));
    }
    answer.push_change(UartChange::IrqConfigured(VirtualUartIrq {
        outlet,
        bitmap: address,
    }));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/irq_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/irq_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/irq_shadow_tests.rs"]
mod shadow_tests;

#[cfg(test)]
#[path = "tests/irq_real_mode_size_tests.rs"]
mod real_mode_size_tests;
