//! NV2A FIFO command decoder and method dispatch table.
//!
//! [`decode_header`] turns a host-endian u32 FIFO word into an
//! [`NvMethodHeader`]; the advance pass consumes the declared
//! argument count and looks up the handler in [`NvMethodTable`].
//! Unregistered methods take the advance pass's unknown-method
//! fallback. Constants live in `cellgov_ps3_abi::rsx_nv_hardware`;
//! this module is the decode and dispatch surface.

use crate::rsx::RsxFifoCursor;
use cellgov_effects::Effect;
use cellgov_time::GuestTicks;
use std::collections::BTreeMap;

pub use cellgov_ps3_abi::rsx_nv_hardware::{
    GCM_FLIP_COMMAND, NV406E_SEMAPHORE_ACQUIRE, NV406E_SEMAPHORE_OFFSET, NV406E_SEMAPHORE_RELEASE,
    NV406E_SET_REFERENCE, NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE, NV4097_GET_REPORT,
    NV4097_NO_OPERATION, NV4097_REPORT_OFFSET_MASK, NV4097_SET_SEMAPHORE_OFFSET,
    NV_CALL_OFFSET_MASK, NV_COUNT_MASK_11, NV_COUNT_SHIFT, NV_FLAG_CALL, NV_FLAG_JUMP,
    NV_FLAG_NEW_JUMP, NV_FLAG_NON_INCREMENT, NV_FLAG_RETURN, NV_METHOD_MASK,
    NV_NEW_JUMP_OFFSET_MASK, NV_OLD_JUMP_OFFSET_MASK,
};

/// Catch-all for non-normal-method bits after every control-flow
/// classifier has failed. Bit 16 sits in this mask (alongside bit 17
/// for RETURN) because a set bit 16 alone would otherwise pass as a
/// normal-method header with a bogus address; RPCS3's
/// `RSX_METHOD_NON_METHOD_CMD_MASK` includes it for the same reason.
const NON_METHOD_MASK: u32 =
    0x8000_0000 | NV_FLAG_JUMP | NV_FLAG_RETURN | 0x0001_0000 | NV_FLAG_NEW_JUMP | NV_FLAG_CALL;

/// Decoded command class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvCommandKind {
    /// Each argument increments the method address by 4.
    Increment,
    /// All arguments write to the same method address.
    NonIncrement,
    /// Sony's JUMP form; 29-bit byte offset.
    Jump {
        /// Absolute byte offset into the FIFO target buffer.
        offset: u32,
    },
    /// RPCS3's "new" JUMP form; 30-bit byte offset. libgcm does not
    /// emit this; classified defensively.
    NewJump {
        /// Absolute byte offset, 30-bit range.
        offset: u32,
    },
    /// CALL; return address pushed on the RSX call stack.
    Call {
        /// Subroutine entry byte offset into the FIFO target buffer.
        offset: u32,
    },
    /// RSX RETURN: pops the call stack to resume the caller.
    Return,
    /// Header that matched no recognised pattern. The raw word is
    /// preserved so downstream diagnostics can distinguish cause
    /// classes (RETURN-with-stray-bits, bit-31-alone, etc.).
    Malformed {
        /// Original header word; preserved verbatim for diagnostics.
        raw: u32,
    },
}

/// Decoded NV method header. `method` and `count` are zero for the
/// control-flow variants of [`NvCommandKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvMethodHeader {
    /// Classified command form (Increment / NonIncrement / Jump / Call / Return / Malformed).
    pub kind: NvCommandKind,
    /// Method byte address (bits 2..=15, 4-byte aligned).
    pub method: u16,
    /// Number of u32 argument dwords following the header (0..=2047).
    pub count: u16,
}

impl NvMethodHeader {
    #[inline]
    const fn normal(kind: NvCommandKind, method: u16, count: u16) -> Self {
        Self {
            kind,
            method,
            count,
        }
    }

    #[inline]
    const fn control(kind: NvCommandKind) -> Self {
        Self {
            kind,
            method: 0,
            count: 0,
        }
    }
}

/// Decode a host-endian u32 FIFO word into a structured header.
///
/// # Cross-runner contract
///
/// First-recognised-form wins for multi-flag inputs, matching NV2A
/// hardware and RPCS3 byte-for-byte: e.g. CALL|JUMP decodes as CALL
/// with the JUMP bit folded into the offset. Stricter classification
/// would flag guest-side corruption but diverge from the RPCS3
/// oracle. RETURN is the only form classified strictly (exact
/// `cmd == NV_FLAG_RETURN`) because it carries no offset or count,
/// so there is nothing legitimate to reject.
///
/// `count` is returned verbatim from bits 18..=28; arity validation
/// (a method receiving fewer args than it semantically requires) is
/// the handler's job, not the decoder's.
pub const fn decode_header(cmd: u32) -> NvMethodHeader {
    if cmd == NV_FLAG_RETURN {
        return NvMethodHeader::control(NvCommandKind::Return);
    }
    if (cmd & 0x0000_0003) == NV_FLAG_CALL {
        return NvMethodHeader::control(NvCommandKind::Call {
            offset: cmd & NV_CALL_OFFSET_MASK,
        });
    }
    if (cmd & 0xE000_0003) == NV_FLAG_NEW_JUMP {
        return NvMethodHeader::control(NvCommandKind::NewJump {
            offset: cmd & NV_NEW_JUMP_OFFSET_MASK,
        });
    }
    if (cmd & 0xE000_0003) == NV_FLAG_JUMP {
        return NvMethodHeader::control(NvCommandKind::Jump {
            offset: cmd & NV_OLD_JUMP_OFFSET_MASK,
        });
    }
    if (cmd & NON_METHOD_MASK) != 0 {
        return NvMethodHeader::control(NvCommandKind::Malformed { raw: cmd });
    }
    let kind = if (cmd & NV_FLAG_NON_INCREMENT) != 0 {
        NvCommandKind::NonIncrement
    } else {
        NvCommandKind::Increment
    };
    let method = (cmd & NV_METHOD_MASK) as u16;
    let count = ((cmd >> NV_COUNT_SHIFT) & NV_COUNT_MASK_11) as u16;
    NvMethodHeader::normal(kind, method, count)
}

/// Mutable state handed to every NV method handler. Built once per
/// FIFO header by the drain and reused across that header's handler
/// dispatches.
pub struct NvDispatchContext<'a> {
    /// Drain-owned FIFO cursor; handlers read/write reference and
    /// flip state via this rather than the runtime directly.
    pub cursor: &'a mut RsxFifoCursor,
    /// Transient label-write offset written by
    /// `NV406E_SEMAPHORE_OFFSET` and consumed by the next
    /// `NV406E_SEMAPHORE_RELEASE`. Folded into the runtime sync-state
    /// hash so a forgotten reset surfaces as a state-hash diff rather
    /// than a silent cross-drain leak.
    pub sem_offset: &'a mut u32,
    /// FIFO-order sink for effects the drain forwards into the next
    /// commit batch.
    pub emitted: &'a mut Vec<Effect>,
    /// Frozen for the duration of one drain; advances by
    /// `consumed_cost` per commit elsewhere.
    pub now: GuestTicks,
}

/// Handler signature for a registered NV method. Failures are
/// emitted as effects, not returned -- handler errors are caller-side
/// bugs caught by unit tests, not a runtime condition.
pub type NvMethodHandler = fn(ctx: &mut NvDispatchContext<'_>, args: &[u32]);

/// `NV406E_SEMAPHORE_OFFSET`: store arg into the transient
/// semaphore-offset register the next RELEASE will read.
pub fn nv406e_semaphore_offset(ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
    if let Some(&offset) = args.first() {
        *ctx.sem_offset = offset;
    }
}

/// `NV406E_SEMAPHORE_RELEASE`: emit an [`Effect::RsxLabelWrite`] at
/// the current `sem_offset` with the release value. A release with
/// no prior offset emits at offset 0; the oracle records the
/// CPU-visible outcome of the stream including guest-side bugs.
pub fn nv406e_semaphore_release(ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
    if let Some(&value) = args.first() {
        ctx.emitted.push(Effect::RsxLabelWrite {
            offset: *ctx.sem_offset,
            value,
        });
    }
}

/// `NV406E_SET_REFERENCE`: write the arg into the cursor's
/// `current_reference` slot. Emits no effect; the slot is folded
/// into [`crate::rsx::RsxFifoCursor::state_hash`].
pub fn nv406e_set_reference(ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
    if let Some(&value) = args.first() {
        ctx.cursor.set_reference(value);
    }
}

/// Register the NV406E semaphore offset / release pair. Errors on
/// collision (call sites must propagate, not paper over).
pub fn register_nv406e_label_handlers(
    table: &mut NvMethodTable,
) -> Result<(), DuplicateRegistration> {
    table.register_unique(NV406E_SEMAPHORE_OFFSET, nv406e_semaphore_offset)?;
    table.register_unique(NV406E_SEMAPHORE_RELEASE, nv406e_semaphore_release)?;
    Ok(())
}

/// Register the `NV406E_SET_REFERENCE` handler. Separate from the
/// label pair so a caller can opt into reference-slot semantics
/// without also opting into label writes.
pub fn register_nv406e_reference_handler(
    table: &mut NvMethodTable,
) -> Result<(), DuplicateRegistration> {
    table.register_unique(NV406E_SET_REFERENCE, nv406e_set_reference)?;
    Ok(())
}

/// `GCM_FLIP_COMMAND` (`0xFEAC`, Sony extension): emit
/// [`Effect::RsxFlipRequest`] with the low-byte buffer index. Flip
/// state transitions happen on commit; this handler does not touch
/// runtime flip state because it lives outside the dispatch context.
pub fn nv4097_flip_buffer(ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
    if let Some(&arg) = args.first() {
        ctx.emitted.push(Effect::RsxFlipRequest {
            buffer_index: arg as u8,
        });
    }
}

/// Register the `GCM_FLIP_COMMAND` handler.
pub fn register_nv4097_flip_handler(
    table: &mut NvMethodTable,
) -> Result<(), DuplicateRegistration> {
    table.register_unique(GCM_FLIP_COMMAND, nv4097_flip_buffer)?;
    Ok(())
}

/// Mask for the report offset field of `NV4097_GET_REPORT`'s arg.
/// Widened to the full u32 (vs Sony's low-24) so microtests that
/// run without `cellGcmInit` (label_base = 0) can target their own
/// statics by absolute address, matching the NV406E absolute-offset
/// convention.
pub(crate) const NV4097_REPORT_OFFSET_MASK_U: u32 = NV4097_REPORT_OFFSET_MASK;

/// `NV4097_GET_REPORT` (`0x1800`): write the low 32 bits of
/// guest-ticks as a 4-byte report payload at
/// `label_base + (arg & NV4097_REPORT_OFFSET_MASK)`. The 16-byte
/// envelope retail titles poll is wider; only the timestamp slot is
/// written today.
pub fn nv4097_get_report(ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
    if let Some(&arg) = args.first() {
        let offset = arg & NV4097_REPORT_OFFSET_MASK_U;
        let value = ctx.now.raw() as u32;
        ctx.emitted.push(Effect::RsxLabelWrite { offset, value });
    }
}

/// Register the `NV4097_GET_REPORT` handler.
pub fn register_nv4097_report_handler(
    table: &mut NvMethodTable,
) -> Result<(), DuplicateRegistration> {
    table.register_unique(NV4097_GET_REPORT, nv4097_get_report)?;
    Ok(())
}

/// Undo the inline `cellGcmSetWriteBackEndLabel` byte-0 / byte-2
/// pre-swap. Real PS3 GPU performs the same swap on write; the
/// oracle applies it here to land the same guest-visible bytes.
/// `const fn` so tests can compute expected values without
/// duplicating the bit math.
pub const fn back_end_semaphore_value_swap(value: u32) -> u32 {
    (value & 0xFF00_FF00) | ((value >> 16) & 0xFF) | (((value) & 0xFF) << 16)
}

/// `NV4097_SET_SEMAPHORE_OFFSET`: identical to
/// [`nv406e_semaphore_offset`]; the front-end / back-end variants
/// share `sem_offset` because we do not model NV pipeline stages.
pub fn nv4097_set_semaphore_offset(ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
    if let Some(&offset) = args.first() {
        *ctx.sem_offset = offset;
    }
}

/// `NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE`: emit an
/// [`Effect::RsxLabelWrite`] with the byte-swapped release value (see
/// [`back_end_semaphore_value_swap`]).
pub fn nv4097_back_end_write_semaphore_release(ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
    if let Some(&value) = args.first() {
        ctx.emitted.push(Effect::RsxLabelWrite {
            offset: *ctx.sem_offset,
            value: back_end_semaphore_value_swap(value),
        });
    }
}

/// Register the back-end semaphore offset / release pair.
pub fn register_nv4097_back_end_semaphore_handlers(
    table: &mut NvMethodTable,
) -> Result<(), DuplicateRegistration> {
    table.register_unique(NV4097_SET_SEMAPHORE_OFFSET, nv4097_set_semaphore_offset)?;
    table.register_unique(
        NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE,
        nv4097_back_end_write_semaphore_release,
    )?;
    Ok(())
}

/// Method-address-keyed dispatch table populated at boot by the
/// `register_nv*` helpers above. Unregistered methods take the
/// advance pass's unknown-method fallback.
#[derive(Debug, Default, Clone)]
pub struct NvMethodTable {
    handlers: BTreeMap<u16, NvMethodHandler>,
}

/// Returned by [`NvMethodTable::register_unique`] when a handler is
/// already registered at the same address. The table is unmodified;
/// `prior` is the existing handler.
// fn-pointer equality is unpredictable across codegen units; `prior`
// is a diagnostic pointer only, which is why PartialEq / Eq are not
// derived.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("NV method 0x{method:04x} already registered")]
pub struct DuplicateRegistration {
    /// Method address whose handler was already bound.
    pub method: u16,
    /// Handler that was already installed at `method`.
    pub prior: NvMethodHandler,
}

impl NvMethodTable {
    /// Empty table; populate via [`register`](Self::register) or
    /// [`register_unique`](Self::register_unique).
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler, silently replacing any prior handler at
    /// the same address. Use [`Self::register_unique`] when a
    /// collision should surface.
    ///
    /// Address `0x0000` fires on every NOP (alignment padding in
    /// real FIFO streams); a handler there is almost always a bug.
    #[inline]
    pub fn register(&mut self, method: u16, handler: NvMethodHandler) -> Option<NvMethodHandler> {
        self.handlers.insert(method, handler)
    }

    /// Register a handler expected to be the first at this address.
    /// Returns [`DuplicateRegistration`] on collision, leaving the
    /// table untouched.
    ///
    /// Address `0x0000` has the same NOP caveat as [`Self::register`].
    #[inline]
    pub fn register_unique(
        &mut self,
        method: u16,
        handler: NvMethodHandler,
    ) -> Result<(), DuplicateRegistration> {
        if let Some(&prior) = self.handlers.get(&method) {
            Err(DuplicateRegistration { method, prior })
        } else {
            self.handlers.insert(method, handler);
            Ok(())
        }
    }

    /// `None` is the advance pass's unknown-method fallback.
    #[inline]
    pub fn lookup(&self, method: u16) -> Option<NvMethodHandler> {
        self.handlers.get(&method).copied()
    }

    /// Number of registered handlers.
    #[inline]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// True when no handlers are registered (every method takes the
    /// unknown-method fallback).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// Table populated with the default workspace handler roster.
    /// `register_*` calls only fail on address collision; a fresh
    /// table cannot collide.
    pub fn with_default_handlers() -> Self {
        let mut t = Self::new();
        register_nv406e_label_handlers(&mut t)
            .expect("fresh NvMethodTable cannot collide on NV406E label pair");
        register_nv406e_reference_handler(&mut t)
            .expect("fresh NvMethodTable cannot collide on NV406E_SET_REFERENCE");
        register_nv4097_flip_handler(&mut t)
            .expect("fresh NvMethodTable cannot collide on GCM_FLIP_COMMAND");
        register_nv4097_report_handler(&mut t)
            .expect("fresh NvMethodTable cannot collide on NV4097_GET_REPORT");
        register_nv4097_back_end_semaphore_handlers(&mut t)
            .expect("fresh NvMethodTable cannot collide on NV4097 back-end semaphore pair");
        t
    }
}

#[cfg(test)]
#[path = "tests/method_tests.rs"]
mod tests;
