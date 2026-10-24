//! Commit-boundary FIFO advance pass.
//!
//! Drains from `cursor.get` up to `cursor.put`, dispatching through
//! the [`NvMethodTable`] and pushing emissions into a caller-owned
//! `Vec<Effect>` that the caller forwards into batch N+1; the advance
//! pass cannot observe its own emissions within the same batch.

use crate::rsx::call_stack::CALL_STACK_OVERFLOW_RAW;
use crate::rsx::method::{
    decode_header, NvCommandKind, NvDispatchContext, NvMethodHeader, NvMethodTable,
};
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
    /// Counted in **dispatch attempts**, not headers: a header with
    /// `count = N` and no matching handler contributes `N`, in either
    /// increment form.
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
/// honoring the FIFO consumer's control-flow headers.
///
/// FIFO bytes are interpreted as big-endian u32 (PS3 ABI). Emissions
/// are appended to `emitted` in FIFO address order. Every header
/// dispatches once per arg with a one-word slice: an Increment header
/// against `method + 4*i`, a NonIncrement header against `method` each
/// time, and a `count == 0` header not at all. The reference console's
/// `rsx_fifo_forms` capture shows a non-increment SET_REFERENCE of three
/// words leaving the third in the register.
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
    label_base: u32,
    call_stack: &mut RsxCallStack,
    table: &NvMethodTable,
    emitted: &mut Vec<Effect>,
    now: GuestTicks,
) -> RsxAdvanceOutcome {
    let mut walk = FifoWalk {
        memory,
        iomap,
        cursor,
        sem_offset,
        label_base,
        call_stack,
        table,
        emitted,
        now,
        iterations: 0,
    };
    let mut outcome = RsxAdvanceOutcome {
        methods_dispatched: 0,
        methods_unknown: 0,
        set_references_dispatched: 0,
        stop: RsxAdvanceStop::Reached,
    };
    loop {
        let get = walk.cursor.get();
        if get == walk.cursor.put() && walk.call_stack.is_empty() {
            break;
        }
        if let Err(stop) = walk.advance_one(get, &mut outcome) {
            outcome.stop = stop;
            return outcome;
        }
    }
    outcome
}

/// The state one `rsx_advance` call walks the FIFO with.
struct FifoWalk<'a> {
    memory: &'a GuestMemory,
    iomap: &'a IoMap,
    cursor: &'a mut RsxFifoCursor,
    sem_offset: &'a mut u32,
    label_base: u32,
    call_stack: &'a mut RsxCallStack,
    table: &'a NvMethodTable,
    emitted: &'a mut Vec<Effect>,
    now: GuestTicks,
    iterations: u32,
}

impl FifoWalk<'_> {
    /// Run the header at `get`. Dispatch counts land in `outcome` before
    /// a later header's refusal, so the error path keeps them.
    #[inline]
    fn advance_one(
        &mut self,
        get: u32,
        outcome: &mut RsxAdvanceOutcome,
    ) -> Result<(), RsxAdvanceStop> {
        // Per-iteration `get > put` check catches a redirect that
        // lands past the tail; an entry-only check would let the
        // walker keep reading until malformed or the iteration cap.
        if get > self.cursor.put() {
            return Err(RsxAdvanceStop::WrappedCursor);
        }
        if self.iterations >= RSX_ADVANCE_ITERATION_CAP {
            return Err(RsxAdvanceStop::Malformed {
                raw: RSX_ADVANCE_ITERATION_CAP_RAW,
            });
        }
        self.iterations += 1;
        let header_word =
            read_fifo_word(self.memory, self.iomap, get).ok_or(RsxAdvanceStop::HeaderOutOfRange)?;
        let header = decode_header(header_word);
        match header.kind {
            NvCommandKind::Increment | NvCommandKind::NonIncrement => {
                self.run_method(get, header, outcome)
            }
            NvCommandKind::Jump { offset } | NvCommandKind::NewJump { offset } => {
                // Unconditional jump; no return-address push.
                // `decode_header` normalizes the offset masks for
                // both Jump and NewJump.
                self.cursor.set_get(offset);
                Ok(())
            }
            NvCommandKind::Call { offset } => {
                let return_addr = get.checked_add(4).ok_or(RsxAdvanceStop::AddressOverflow)?;
                if self.call_stack.push(return_addr).is_err() {
                    return Err(RsxAdvanceStop::Malformed {
                        raw: CALL_STACK_OVERFLOW_RAW,
                    });
                }
                self.cursor.set_get(offset);
                Ok(())
            }
            NvCommandKind::Return => {
                let return_addr = self
                    .call_stack
                    .pop()
                    .map_err(|_| RsxAdvanceStop::Malformed {
                        raw: RSX_ADVANCE_UNDERFLOW_RAW,
                    })?;
                self.cursor.set_get(return_addr);
                Ok(())
            }
            NvCommandKind::Malformed { raw } => Err(RsxAdvanceStop::Malformed { raw }),
        }
    }

    /// Read an Increment or NonIncrement header's arguments, move `get`
    /// past them, then dispatch each one.
    #[inline]
    fn run_method(
        &mut self,
        get: u32,
        header: NvMethodHeader,
        outcome: &mut RsxAdvanceOutcome,
    ) -> Result<(), RsxAdvanceStop> {
        let count = header.count as u32;
        let next_get = method_span(get, count, self.cursor.put())?;
        let args = read_method_args(self.memory, self.iomap, get, count)?;
        self.cursor.set_get(next_get);
        let mut ctx = NvDispatchContext {
            cursor: &mut *self.cursor,
            sem_offset: &mut *self.sem_offset,
            label_base: self.label_base,
            emitted: &mut *self.emitted,
            now: self.now,
        };
        let step = if header.kind == NvCommandKind::Increment {
            4
        } else {
            0
        };
        for (i, arg) in args.iter().enumerate() {
            let sub_method = header.method.wrapping_add((i as u16).wrapping_mul(step));
            dispatch_method(
                self.table,
                sub_method,
                std::slice::from_ref(arg),
                &mut ctx,
                outcome,
            );
        }
        Ok(())
    }
}

/// Return the `get` past a method header at `get` and its `count`
/// argument words, refusing a span that overflows u32 or runs past
/// `put`.
#[inline]
fn method_span(get: u32, count: u32, put: u32) -> Result<u32, RsxAdvanceStop> {
    let total_bytes = count
        .checked_mul(4)
        .and_then(|b| b.checked_add(4))
        .ok_or(RsxAdvanceStop::AddressOverflow)?;
    let next_get = get
        .checked_add(total_bytes)
        .ok_or(RsxAdvanceStop::AddressOverflow)?;
    if next_get > put {
        return Err(RsxAdvanceStop::TruncatedMethod);
    }
    Ok(next_get)
}

/// Read the `count` argument words that follow the method header at
/// `get`.
#[inline]
fn read_method_args(
    memory: &GuestMemory,
    iomap: &IoMap,
    get: u32,
    count: u32,
) -> Result<Vec<u32>, RsxAdvanceStop> {
    let args_start = get.wrapping_add(4);
    let mut args: Vec<u32> = Vec::with_capacity(count as usize);
    for i in 0..count {
        let arg_addr = args_start.wrapping_add(i.wrapping_mul(4));
        let word = read_fifo_word(memory, iomap, arg_addr).ok_or(RsxAdvanceStop::ArgOutOfRange)?;
        args.push(word);
    }
    Ok(args)
}

#[cfg(test)]
#[path = "tests/advance_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/hardware_trace_tests.rs"]
mod hardware_trace_tests;

#[cfg(test)]
#[path = "tests/fifo_forms_console_tests.rs"]
mod fifo_forms_console_tests;
