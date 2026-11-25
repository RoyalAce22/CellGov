//! The virtual UART's interrupt shadow: one bit for each port, which lv1's
//! status function sets while the port's status and mask share a bit and
//! clears otherwise; see [`cellgov_ps3_abi::lv1::uart`].
//!
//! lv1 keeps the shadow at +56 of the partition's virtual UART object and
//! stores each changed doubleword at the address that +48 holds: the
//! bitmap of `configure_virtual_uart_irq`, or the object's own doublewords
//! before that call. When a raise takes the count of set bits from 0 to
//! 1, lv1 gives the irq outlet to the event function. So a port's
//! interrupt reaches the thread that the outlet's plug names.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::uart::{virtual_uart_shadow_bit, VIRTUAL_UART_IRQ_BITMAP_BYTES};

use super::set_param::VirtualUartPortParams;
use super::status::{self, ShadowStep};
use super::{UartChange, UartState};
use crate::answer::{HypercallAnswer, UnmodelledError};
use crate::group::interrupt::raise::{self, RaiseState};
use crate::request::HypercallRequest;

/// The number of doublewords in the virtual UART's shadow.
pub const UART_SHADOW_DOUBLEWORDS: usize = (VIRTUAL_UART_IRQ_BITMAP_BYTES / 8) as usize;

/// The bytes of one shadow doubleword.
const DOUBLEWORD_BYTES: u64 = std::mem::size_of::<u64>() as u64;

/// One doubleword of the virtual UART's shadow, as a call leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartShadowWrite {
    /// The doubleword, below [`UART_SHADOW_DOUBLEWORDS`].
    pub index: u8,
    /// The doubleword's new value.
    pub value: u64,
}

/// The virtual UART's shadow. Fields 1 to 4 are its doublewords, in order.
pub(super) struct VirtualUartShadow(pub(super) [u64; UART_SHADOW_DOUBLEWORDS]);

impl LaneValue for VirtualUartShadow {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        for (field, &doubleword) in (1u8..).zip(&self.0) {
            lanes.lane(field, 0, doubleword);
        }
    }
}

/// The state that a status step reads.
pub(crate) struct StatusInput<'a> {
    /// The virtual UART group's state.
    pub uart: &'a UartState,
    /// The interrupt state that the outlet's event reads. Its storage is
    /// the caller's real address space, which holds the bitmaps.
    pub raise: RaiseState<'a>,
}

/// Adds lv1's status step for the port `params.port` to `answer`, when
/// its receive buffer holds `received` bytes after the call.
///
/// The step follows [`status::shadow_step`]:
/// - a raise sets the port's bit, stores the doubleword, and gives the
///   irq outlet to the event function when no other bit was set
///   [OFW-4.93 lv1 a:0x2F225C-0x2F2280] [OFW-4.93 lv1 a:0x2F2394-0x2F239C];
/// - a clear clears the bit and stores the doubleword
///   [OFW-4.93 lv1 a:0x2F2308-0x2F2334].
///
/// A call can run the status function more than once. Each earlier run
/// leaves the bit as the state before the call has it, so only the last
/// run changes it.
///
/// # Errors
///
/// Returns the refusal text in these cases:
/// - [`status::shadow_step`] refuses;
/// - CellGov holds no storage for the doubleword in the bitmap;
/// - the outlet's event refuses; see [`raise::event`].
pub(super) fn status_step(
    request: &HypercallRequest,
    firmware: &str,
    input: &StatusInput<'_>,
    params: &VirtualUartPortParams,
    received: u64,
    answer: &mut HypercallAnswer,
) -> Result<(), UnmodelledError> {
    let shadow = input.uart.shadow();
    let port = u8::try_from(params.port)
        .map_err(|_| UnmodelledError::new("a virtual UART port above the ports of the shadow"))?;
    let (doubleword, bit) = virtual_uart_shadow_bit(port);
    let index = doubleword as u8;
    let old = shadow[usize::from(index)];
    let value = match status::shadow_step(firmware, params, received, old & bit != 0)? {
        ShadowStep::Keep => return Ok(()),
        ShadowStep::Raise => old | bit,
        ShadowStep::Clear => old & !bit,
    };
    answer.push_change(UartChange::ShadowSet(VirtualUartShadowWrite {
        index,
        value,
    }));
    let Some(irq) = input.uart.irq() else {
        return Ok(());
    };
    let range = irq
        .bitmap
        .checked_add(u64::from(index) * DOUBLEWORD_BYTES)
        .and_then(|address| ByteRange::new(GuestAddr::new(address), DOUBLEWORD_BYTES))
        .filter(|range| {
            input
                .raise
                .storage
                .holds(range.start().raw(), range.length())
        });
    let Some(range) = range else {
        return Err(UnmodelledError::new(
            "a virtual UART interrupt bitmap that CellGov holds no storage for",
        ));
    };
    answer.effects.push(Effect::SharedWriteIntent {
        range,
        bytes: WritePayload::new(value.to_be_bytes().to_vec()),
        source: request.unit,
        source_time: request.tick,
    });
    let first_bit = shadow == [0; UART_SHADOW_DOUBLEWORDS] && value != 0;
    if first_bit {
        raise::event(request, &input.raise, irq.outlet, answer)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/shadow_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/shadow_hash_tests.rs"]
mod hash_tests;
