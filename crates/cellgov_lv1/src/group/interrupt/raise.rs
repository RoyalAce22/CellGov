//! lv1's event function, its raise of a plug's bit, and its test of a
//! thread's status shadow against the thread's mask.
//!
//! An event on a port or irq outlet sets the object's pending byte, and
//! raises the plug's bit in the status shadow of the plug's thread; see
//! [`cellgov_ps3_abi::lv1::send_event`]. `send_event_locally` and the
//! virtual UART's receive interrupt give an event this way.
//!
//! A raised bit reaches the thread at its next return from lv1: the
//! return ANDs the shadow with the mask; see
//! [`cellgov_ps3_abi::lv1::interrupt_mask`]. A raise on a paused thread
//! wakes it when its mask holds the bit, and the thread then returns.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::{be::read_u64, ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::interrupt::{IRQ_MASK_OFFSET, IRQ_STATUS_BYTES};
use cellgov_ps3_abi::lv1::irq_plug::plug_status_bit;

use super::bitmap::{
    IrqStateBitmapKey, IrqStateBitmapState, StatusShadowWrite, SHADOW_DOUBLEWORDS,
};
use super::event_port::EventReceivePortState;
use super::pause::PausedThreadState;
use super::InterruptChange;
use crate::answer::{HypercallAnswer, UnmodelledError};
use crate::request::HypercallRequest;

/// The bytes of one status or mask doubleword.
const DOUBLEWORD_BYTES: u64 = std::mem::size_of::<u64>() as u64;

/// The interrupt state that an event reads.
pub(crate) struct RaiseState<'a> {
    /// The interrupt state bitmaps and the status shadows.
    pub bitmaps: &'a IrqStateBitmapState,
    /// The event receive ports, the irq outlets and the pending events.
    pub ports: &'a EventReceivePortState,
    /// The paused threads.
    pub paused: &'a PausedThreadState,
    /// The caller's real address space, which holds the bitmaps.
    pub storage: &'a GuestMemory,
}

/// Adds lv1's event on the port or irq outlet `id` to `answer`.
///
/// The event function [OFW-4.93 lv1 a:0x2DF808] does these steps:
/// 1. It returns at once while the object's pending byte is set
///    [OFW-4.93 lv1 a:0x2DF864-0x2DF86C].
/// 2. It sets the pending byte [OFW-4.93 lv1 a:0x2DF87C].
/// 3. For a connected object, it raises the plug's bit in the shadow of
///    the plug's thread [OFW-4.93 lv1 a:0x2DF8BC-0x2DF8C8]; see [`raise`].
///
/// `id` names an object that the hypervisor holds.
///
/// # Errors
///
/// Returns the refusal text of [`raise`].
pub(crate) fn event(
    request: &HypercallRequest,
    state: &RaiseState<'_>,
    id: u64,
    answer: &mut HypercallAnswer,
) -> Result<(), UnmodelledError> {
    if state.ports.is_pending(id) {
        return Ok(());
    }
    answer.push_change(InterruptChange::EventPending(id));
    let connected = state
        .ports
        .port(id)
        .or_else(|| state.ports.outlet(id))
        .and_then(|object| object.connection.map(|key| (key, object.plug)));
    match connected {
        Some((key, plug)) => raise(request, state, key, plug, answer),
        None => Ok(()),
    }
}

/// Adds lv1's raise of the plug `plug` of the thread `key` to `answer`.
///
/// The raise function [OFW-4.93 lv1 a:0x2C65B8] does these steps:
/// 1. It changes nothing for a bit that the shadow holds
///    [OFW-4.93 lv1 a:0x2C65F8-0x2C661C].
/// 2. It sets the bit in the shadow, and stores the shadow doubleword in
///    the thread's bitmap [OFW-4.93 lv1 a:0x2C6620-0x2C6634].
/// 3. For a paused thread, it ANDs the bit with the mask doubleword
///    [OFW-4.93 lv1 a:0x2C6628] [OFW-4.93 lv1 a:0x2C6680-0x2C6690]. A
///    nonzero result clears the pause flag and runs the wake list
///    [OFW-4.93 lv1 a:0x2C6694-0x2C66DC]. The answer then carries the
///    wake.
///
/// # Errors
///
/// Returns the refusal text for these inputs:
/// - a plug of a running thread other than the caller, which lv1
///   signals with an interprocessor interrupt
///   [OFW-4.93 lv1 a:0x2C663C-0x2C6650] [OFW-4.93 lv1 a:0x2C6720-0x2C672C];
/// - a thread with no registered bitmap, whose shadow lv1 copies to its
///   own doublewords;
/// - a bitmap whose status or mask doubleword CellGov holds no storage
///   for.
pub(crate) fn raise(
    request: &HypercallRequest,
    state: &RaiseState<'_>,
    key: IrqStateBitmapKey,
    plug: u8,
    answer: &mut HypercallAnswer,
) -> Result<(), UnmodelledError> {
    let (doubleword, bit) = plug_status_bit(plug);
    let index = doubleword as u8;
    let shadow = state.bitmaps.shadow(key)[usize::from(index)];
    if shadow & bit != 0 {
        return Ok(());
    }
    let paused = state.paused.thread(key.thread).is_some();
    if request.thread != Some(key.thread) && !paused {
        return Err(UnmodelledError::new(
            "an event on a plug of a running thread, which lv1 signals with an interprocessor interrupt",
        ));
    }
    let Some(bitmap) = state.bitmaps.bitmap(key) else {
        return Err(UnmodelledError::new(
            "an event on a plug of a thread that registered no interrupt state bitmap",
        ));
    };
    let range = |offset: u64| {
        bitmap
            .checked_add(offset + u64::from(index) * DOUBLEWORD_BYTES)
            .and_then(|address| ByteRange::new(GuestAddr::new(address), DOUBLEWORD_BYTES))
    };
    let status = range(0).filter(|range| state.storage.holds(range.start().raw(), range.length()));
    let mask = range(IRQ_MASK_OFFSET).and_then(|range| state.storage.read(range));
    let (Some(status), Some(mask)) = (status, mask) else {
        return Err(UnmodelledError::new(
            "an event on a plug whose status and mask doublewords CellGov holds no storage for",
        ));
    };
    let value = shadow | bit;
    answer.effects.push(Effect::SharedWriteIntent {
        range: status,
        bytes: WritePayload::new(value.to_be_bytes().to_vec()),
        source: request.unit,
        source_time: request.tick,
    });
    answer.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key,
        index,
        value,
    }));
    if paused && read_u64(&mask, 0) & bit != 0 {
        answer.push_change(InterruptChange::ThreadWoken(key.thread));
    }
    Ok(())
}

/// Adds lv1's clear of the plug `plug` of the thread `key` to `answer`.
///
/// The clear function [OFW-4.93 lv1 a:0x2C3794] does these steps:
/// 1. It clears the plug's bit in the shadow
///    [OFW-4.93 lv1 a:0x2C37C0-0x2C37D4]. The answer carries the cleared
///    doubleword when the bit was set.
/// 2. It stores the shadow doubleword through +1728 of the thread's
///    context [OFW-4.93 lv1 a:0x2C37D8]. The answer writes it to the
///    thread's registered bitmap. For a thread with no bitmap, lv1 stores
///    it in its own doublewords, and the answer writes nothing.
///
/// # Errors
///
/// Returns the refusal text, and changes nothing, when CellGov holds no
/// storage for the status doubleword of the thread's bitmap.
pub(crate) fn clear(
    request: &HypercallRequest,
    bitmaps: &IrqStateBitmapState,
    storage: &GuestMemory,
    key: IrqStateBitmapKey,
    plug: u8,
    answer: &mut HypercallAnswer,
) -> Result<(), UnmodelledError> {
    let (doubleword, bit) = plug_status_bit(plug);
    let index = doubleword as u8;
    let shadow = bitmaps.shadow(key)[usize::from(index)];
    let cleared = shadow & !bit;
    if let Some(bitmap) = bitmaps.bitmap(key) {
        let range = bitmap
            .checked_add(doubleword * DOUBLEWORD_BYTES)
            .and_then(|address| ByteRange::new(GuestAddr::new(address), DOUBLEWORD_BYTES))
            .filter(|range| storage.holds(range.start().raw(), range.length()))
            .ok_or(UnmodelledError::new(
                "an IRQ plug whose status doubleword CellGov holds no storage for",
            ))?;
        answer.effects.push(Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(cleared.to_be_bytes().to_vec()),
            source: request.unit,
            source_time: request.tick,
        });
    }
    if cleared != shadow {
        answer.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
            key,
            index,
            value: cleared,
        }));
    }
    Ok(())
}

/// Whether the status shadow of the thread `key` holds a bit that the
/// thread's mask holds: the test of lv1's return
/// [OFW-4.93 lv1 a:0x0FC4-0x1010], of its pause step
/// [OFW-4.93 lv1 a:0x2C6930-0x2C695C] and of its wake check
/// [OFW-4.93 lv1 a:0x2C6B40-0x2C6B70].
///
/// The mask doublewords follow the status doublewords of the thread's
/// bitmap; see [`cellgov_ps3_abi::lv1::interrupt`]. A shadow of 0 holds
/// no such bit, and the test reads no mask.
///
/// # Errors
///
/// Returns the refusal text for a shadow with a set bit in these cases:
/// - the thread registered no bitmap, so lv1 reads its own mask
///   doublewords, which CellGov does not hold;
/// - CellGov holds no storage for the mask doublewords.
pub(crate) fn unmasked(
    bitmaps: &IrqStateBitmapState,
    key: IrqStateBitmapKey,
    storage: &GuestMemory,
) -> Result<bool, UnmodelledError> {
    let shadow = bitmaps.shadow(key);
    if shadow == [0; SHADOW_DOUBLEWORDS] {
        return Ok(false);
    }
    let Some(bitmap) = bitmaps.bitmap(key) else {
        return Err(UnmodelledError::new(
            "a raised status shadow bit of a thread that registered no interrupt state bitmap, \
             whose mask lv1 keeps in its own doublewords",
        ));
    };
    let mask = bitmap
        .checked_add(IRQ_MASK_OFFSET)
        .and_then(|address| ByteRange::new(GuestAddr::new(address), IRQ_STATUS_BYTES))
        .and_then(|range| storage.read(range));
    let Some(mask) = mask else {
        return Err(UnmodelledError::new(
            "a raised status shadow bit of a thread whose mask doublewords CellGov holds no \
             storage for",
        ));
    };
    Ok((0..SHADOW_DOUBLEWORDS)
        .zip(shadow)
        .any(|(index, doubleword)| doubleword & read_u64(&mask, index * 8) != 0))
}

#[cfg(test)]
#[path = "tests/raise_tests.rs"]
mod tests;
