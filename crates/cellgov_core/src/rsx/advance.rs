//! Commit-boundary FIFO advance pass.
//!
//! Drains from `cursor.get` up to `cursor.put`, dispatching through
//! the [`NvMethodTable`] and pushing emissions into a caller-owned
//! `Vec<Effect>` that the caller forwards into batch N+1; the advance
//! pass cannot observe its own emissions within the same batch.

use crate::rsx::call_stack::CALL_STACK_OVERFLOW_RAW;
use crate::rsx::method::{decode_header, NvCommandKind, NvDispatchContext, NvMethodTable};
use crate::rsx::{IoMap, RsxCallStack, RsxFifoCursor};
use cellgov_effects::Effect;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::GuestTicks;

/// Iteration cap on a single `rsx_advance` invocation. After this
/// many header dispatches without reaching `cursor.put()` the
/// walker rejects with [`RsxAdvanceStop::Malformed`].
pub const RSX_ADVANCE_ITERATION_CAP: u32 = 1_000_000;

/// Synthetic raw word emitted by `Malformed { raw }` when the
/// iteration cap trips.
pub const RSX_ADVANCE_ITERATION_CAP_RAW: u32 = 0x4000_01FF;

/// Synthetic raw word emitted by `Malformed { raw }` when a Return
/// header is decoded with the call stack empty.
pub const RSX_ADVANCE_UNDERFLOW_RAW: u32 = 0x4000_02FF;

/// Terminal condition of a single advance-pass call.
///
/// Every non-`Reached` variant parks `cursor.get` at the offending header
/// (or leaves it unchanged on entry-condition failures) so a caller can
/// re-enter safely once the condition is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsxAdvanceStop {
    /// Drain reached `cursor.put` cleanly.
    Reached,
    /// Decoder rejected the header. Synthetic raw words
    /// `CALL_STACK_OVERFLOW_RAW`, `RSX_ADVANCE_ITERATION_CAP_RAW`,
    /// and `RSX_ADVANCE_UNDERFLOW_RAW` distinguish runtime errors
    /// from a real malformed-header rejection.
    Malformed {
        /// Raw 32-bit header word.
        raw: u32,
    },
    /// Header read fell in unmapped memory; `get` unchanged.
    HeaderOutOfRange,
    /// Argument-word read fell in unmapped memory; `get` parked at the header.
    ArgOutOfRange,
    /// Declared arg count would advance past `cursor.put`.
    TruncatedMethod,
    /// `get + header + args` would overflow u32.
    AddressOverflow,
    /// `cursor.get > cursor.put`. On a redirect-induced wrap `get`
    /// parks at the redirect target; on entry refusal `get` is
    /// unchanged.
    WrappedCursor,
}

/// Summary of one advance-pass invocation. Counters saturate at `u32::MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxAdvanceOutcome {
    /// Dispatches that hit a registered handler.
    pub methods_dispatched: u32,
    /// Counted in **dispatch attempts**, not headers: an Increment header
    /// with `count = N` and no matching handler contributes `N`; a
    /// NonIncrement header contributes `1` regardless of `count`.
    pub methods_unknown: u32,
    /// `NV406E_SET_REFERENCE` dispatches inside this pass; the
    /// cursor's `current_reference` slot only retains the final value.
    pub set_references_dispatched: u32,
    /// Terminal condition.
    pub stop: RsxAdvanceStop,
}

impl RsxAdvanceOutcome {
    /// Whether `stop == RsxAdvanceStop::Reached`.
    #[inline]
    pub fn reached_put(&self) -> bool {
        matches!(self.stop, RsxAdvanceStop::Reached)
    }
}

fn read_fifo_word(memory: &GuestMemory, iomap: &IoMap, io_offset: u32) -> Option<u32> {
    let ea = iomap.translate(io_offset)?;
    let range = ByteRange::new(GuestAddr::new(ea as u64), 4)?;
    let bytes = memory.read(range)?;
    let word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    Some(word)
}

fn dispatch_method(
    table: &NvMethodTable,
    method: u16,
    args: &[u32],
    ctx: &mut NvDispatchContext<'_>,
    outcome: &mut RsxAdvanceOutcome,
) {
    match table.lookup(method) {
        Some(handler) => {
            handler(ctx, args);
            outcome.methods_dispatched = outcome.methods_dispatched.saturating_add(1);
            if method == crate::rsx::method::NV406E_SET_REFERENCE {
                outcome.set_references_dispatched =
                    outcome.set_references_dispatched.saturating_add(1);
            }
        }
        None => {
            outcome.methods_unknown = outcome.methods_unknown.saturating_add(1);
        }
    }
}

/// Drain the FIFO from `cursor.get()` up to `cursor.put()`,
/// honoring 40F-consumer control-flow headers.
///
/// FIFO bytes are interpreted as big-endian u32 (PS3 ABI). Emissions
/// are appended to `emitted` in FIFO address order. Increment headers
/// dispatch once per arg against `method + 4*i`; NonIncrement headers
/// dispatch once with the full slice (including the empty slice for
/// `count == 0`), so handlers must index via `args.first()` / `get`.
///
/// Control-flow headers are honored unconditionally: `Call` pushes,
/// `Return` pops, `Jump` / `NewJump` redirect without push.
/// `RSX_ADVANCE_ITERATION_CAP` surfaces runaway FIFOs as `Malformed`.
/// The cursor->MMIO writeback lives in `commit_step` gated on
/// `rsx_consume_fifo`.
#[allow(
    clippy::too_many_arguments,
    reason = "Params span three lifetime classes (read-only inputs, persistent \
              mutable state, per-call sink). Bundling into a context struct would \
              obscure that the call stack is a snapshot-captured field while \
              emitted is a per-batch scratch sink."
)]
pub fn rsx_advance(
    memory: &GuestMemory,
    iomap: &IoMap,
    cursor: &mut RsxFifoCursor,
    sem_offset: &mut u32,
    call_stack: &mut RsxCallStack,
    table: &NvMethodTable,
    emitted: &mut Vec<Effect>,
    now: GuestTicks,
) -> RsxAdvanceOutcome {
    let mut outcome = RsxAdvanceOutcome {
        methods_dispatched: 0,
        methods_unknown: 0,
        set_references_dispatched: 0,
        stop: RsxAdvanceStop::Reached,
    };
    let mut iterations: u32 = 0;
    loop {
        let get = cursor.get();
        if get == cursor.put() && call_stack.is_empty() {
            break;
        }
        // Per-iteration `get > put` check catches a redirect that
        // lands past the tail; an entry-only check would let the
        // walker keep reading until malformed or the iteration cap.
        if get > cursor.put() {
            outcome.stop = RsxAdvanceStop::WrappedCursor;
            return outcome;
        }
        if iterations >= RSX_ADVANCE_ITERATION_CAP {
            outcome.stop = RsxAdvanceStop::Malformed {
                raw: RSX_ADVANCE_ITERATION_CAP_RAW,
            };
            return outcome;
        }
        iterations += 1;
        let header_word = match read_fifo_word(memory, iomap, get) {
            Some(w) => w,
            None => {
                outcome.stop = RsxAdvanceStop::HeaderOutOfRange;
                return outcome;
            }
        };
        let header = decode_header(header_word);
        match header.kind {
            NvCommandKind::Increment | NvCommandKind::NonIncrement => {
                let count = header.count as u32;
                let Some(total_bytes_u32) = count.checked_mul(4).and_then(|b| b.checked_add(4))
                else {
                    outcome.stop = RsxAdvanceStop::AddressOverflow;
                    return outcome;
                };
                let Some(next_get) = get.checked_add(total_bytes_u32) else {
                    outcome.stop = RsxAdvanceStop::AddressOverflow;
                    return outcome;
                };
                if next_get > cursor.put() {
                    outcome.stop = RsxAdvanceStop::TruncatedMethod;
                    return outcome;
                }
                let args_start = get.wrapping_add(4);
                let mut args: Vec<u32> = Vec::with_capacity(count as usize);
                for i in 0..count {
                    let arg_addr = args_start.wrapping_add(i.wrapping_mul(4));
                    match read_fifo_word(memory, iomap, arg_addr) {
                        Some(w) => args.push(w),
                        None => {
                            outcome.stop = RsxAdvanceStop::ArgOutOfRange;
                            return outcome;
                        }
                    }
                }
                cursor.set_get(next_get);
                let mut ctx = NvDispatchContext {
                    cursor: &mut *cursor,
                    sem_offset: &mut *sem_offset,
                    emitted: &mut *emitted,
                    now,
                };
                match header.kind {
                    NvCommandKind::Increment => {
                        for (i, arg) in args.iter().enumerate() {
                            let sub_method = header.method.wrapping_add((i as u16) * 4);
                            dispatch_method(
                                table,
                                sub_method,
                                std::slice::from_ref(arg),
                                &mut ctx,
                                &mut outcome,
                            );
                        }
                    }
                    NvCommandKind::NonIncrement => {
                        dispatch_method(table, header.method, &args, &mut ctx, &mut outcome);
                    }
                    _ => unreachable!("outer match guards Increment / NonIncrement only"),
                }
            }
            NvCommandKind::Jump { offset } | NvCommandKind::NewJump { offset } => {
                // Unconditional jump; no return-address push.
                // `decode_header` normalizes the offset masks for
                // both Jump and NewJump.
                cursor.set_get(offset);
            }
            NvCommandKind::Call { offset } => {
                let Some(return_addr) = get.checked_add(4) else {
                    outcome.stop = RsxAdvanceStop::AddressOverflow;
                    return outcome;
                };
                if call_stack.push(return_addr).is_err() {
                    outcome.stop = RsxAdvanceStop::Malformed {
                        raw: CALL_STACK_OVERFLOW_RAW,
                    };
                    return outcome;
                }
                cursor.set_get(offset);
            }
            NvCommandKind::Return => match call_stack.pop() {
                Ok(return_addr) => {
                    cursor.set_get(return_addr);
                }
                Err(_) => {
                    outcome.stop = RsxAdvanceStop::Malformed {
                        raw: RSX_ADVANCE_UNDERFLOW_RAW,
                    };
                    return outcome;
                }
            },
            NvCommandKind::Malformed { raw } => {
                outcome.stop = RsxAdvanceStop::Malformed { raw };
                return outcome;
            }
        }
    }
    outcome
}

#[cfg(test)]
#[path = "tests/advance_tests.rs"]
mod tests;
