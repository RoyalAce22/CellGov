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
mod tests {
    use super::*;
    use crate::rsx::call_stack::CALL_STACK_DEPTH;
    use crate::rsx::method::{
        register_nv406e_label_handlers, register_nv406e_reference_handler, NV406E_SEMAPHORE_OFFSET,
        NV406E_SEMAPHORE_RELEASE, NV406E_SET_REFERENCE, NV_COUNT_SHIFT, NV_FLAG_CALL, NV_FLAG_JUMP,
        NV_FLAG_RETURN,
    };
    use cellgov_mem::GuestMemory;

    const FIFO_BASE: u32 = 0x1000;

    fn make_memory() -> GuestMemory {
        GuestMemory::new(0x4000)
    }

    fn encode_header(method: u16, count: u16) -> u32 {
        ((count as u32) << NV_COUNT_SHIFT) | (method as u32)
    }

    fn write_fifo_words(memory: &mut GuestMemory, base: u32, words: &[u32]) {
        let len = (words.len() as u64) * 4;
        let range = ByteRange::new(GuestAddr::new(base as u64), len).unwrap();
        let mut bytes = Vec::with_capacity(words.len() * 4);
        for &w in words {
            bytes.extend_from_slice(&w.to_be_bytes());
        }
        memory.apply_commit(range, &bytes).unwrap();
    }

    fn setup_table() -> NvMethodTable {
        let mut t = NvMethodTable::new();
        register_nv406e_label_handlers(&mut t).unwrap();
        register_nv406e_reference_handler(&mut t).unwrap();
        t
    }

    #[test]
    fn advance_is_noop_when_get_equals_put() {
        let memory = make_memory();
        let mut cursor = RsxFifoCursor::new();
        let mut sem_offset = 0u32;
        let table = NvMethodTable::new();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert_eq!(
            outcome,
            RsxAdvanceOutcome {
                methods_dispatched: 0,
                methods_unknown: 0,
                set_references_dispatched: 0,
                stop: RsxAdvanceStop::Reached,
            }
        );
        assert!(outcome.reached_put());
        assert!(emitted.is_empty());
        assert_eq!(cursor.get(), 0);
    }

    #[test]
    fn advance_dispatches_nv406e_offset_release_pair_and_emits_label_write() {
        let mut memory = make_memory();
        let words = [
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x80u32,
            encode_header(NV406E_SEMAPHORE_RELEASE, 1),
            0x1234_5678u32,
        ];
        write_fifo_words(&mut memory, FIFO_BASE, &words);
        let put = FIFO_BASE + (words.len() as u32) * 4;

        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();

        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert!(outcome.reached_put(), "drain must consume the FIFO");
        assert_eq!(outcome.methods_dispatched, 2);
        assert_eq!(outcome.methods_unknown, 0);
        assert_eq!(cursor.get(), put, "get must advance to put on clean drain");
        assert_eq!(sem_offset, 0x80);
        assert_eq!(
            emitted.as_slice(),
            &[Effect::RsxLabelWrite {
                offset: 0x80,
                value: 0x1234_5678,
            }]
        );
    }

    #[test]
    fn advance_is_deterministic_across_two_runs() {
        fn run() -> (u32, u32, Vec<Effect>) {
            let mut memory = make_memory();
            let words = [
                encode_header(NV406E_SEMAPHORE_OFFSET, 1),
                0xAAu32,
                encode_header(NV406E_SEMAPHORE_RELEASE, 1),
                0xCAFE_BABEu32,
                encode_header(NV406E_SEMAPHORE_OFFSET, 1),
                0xBBu32,
                encode_header(NV406E_SEMAPHORE_RELEASE, 1),
                0xDEAD_BEEFu32,
            ];
            write_fifo_words(&mut memory, FIFO_BASE, &words);
            let put = FIFO_BASE + (words.len() as u32) * 4;
            let mut cursor = RsxFifoCursor::new();
            cursor.set_put(put);
            cursor.set_get(FIFO_BASE);
            let mut sem_offset = 0u32;
            let table = setup_table();
            let mut emitted: Vec<Effect> = Vec::new();
            {
                let mut call_stack = RsxCallStack::new();
                rsx_advance(
                    &memory,
                    &IoMap::IDENTITY,
                    &mut cursor,
                    &mut sem_offset,
                    &mut call_stack,
                    &table,
                    &mut emitted,
                    GuestTicks::ZERO,
                )
            };
            (cursor.get(), sem_offset, emitted)
        }
        let a = run();
        let b = run();
        assert_eq!(a, b);
    }

    #[test]
    fn advance_preserves_fifo_address_order_in_emitted_effects() {
        let mut memory = make_memory();
        let words = [
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x10u32,
            encode_header(NV406E_SEMAPHORE_RELEASE, 1),
            0x01u32,
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x20u32,
            encode_header(NV406E_SEMAPHORE_RELEASE, 1),
            0x02u32,
        ];
        write_fifo_words(&mut memory, FIFO_BASE, &words);
        let put = FIFO_BASE + (words.len() as u32) * 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert_eq!(
            emitted,
            vec![
                Effect::RsxLabelWrite {
                    offset: 0x10,
                    value: 0x01,
                },
                Effect::RsxLabelWrite {
                    offset: 0x20,
                    value: 0x02,
                },
            ]
        );
    }

    #[test]
    fn advance_counts_unknown_methods_and_keeps_draining() {
        let mut memory = make_memory();
        // 2-arg Increment at an unregistered address counts as two
        // unknown dispatches (one per sub-method address).
        let unknown_method: u16 = 0x0200;
        let words = [
            encode_header(unknown_method, 2),
            0xDEAD_BEEFu32,
            0xCAFE_BABEu32,
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x40u32,
            encode_header(NV406E_SEMAPHORE_RELEASE, 1),
            0x99u32,
        ];
        write_fifo_words(&mut memory, FIFO_BASE, &words);
        let put = FIFO_BASE + (words.len() as u32) * 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert!(outcome.reached_put());
        assert_eq!(outcome.methods_unknown, 2);
        assert_eq!(outcome.methods_dispatched, 2);
        assert_eq!(cursor.get(), put);
        assert_eq!(
            emitted.as_slice(),
            &[Effect::RsxLabelWrite {
                offset: 0x40,
                value: 0x99,
            }]
        );
    }

    #[test]
    fn advance_honors_jump_redirect_and_reaches_put_via_target() {
        // 40F consumer: Jump now redirects the walker to the target
        // offset rather than parking. Layout: OFFSET=0x10 at base,
        // JUMP to base+0x20, RELEASE=0x99 at base+0x20. put sits
        // just past RELEASE so the walker reaches put cleanly via
        // the jump target.
        let mut memory = make_memory();
        let jump_target = FIFO_BASE + 0x20;
        let jump_header = NV_FLAG_JUMP | jump_target;
        let prologue = [
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x10u32,
            jump_header,
        ];
        let target_block = [encode_header(NV406E_SEMAPHORE_RELEASE, 1), 0x99u32];
        write_fifo_words(&mut memory, FIFO_BASE, &prologue);
        write_fifo_words(&mut memory, jump_target, &target_block);
        let put = jump_target + (target_block.len() as u32) * 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let mut call_stack = RsxCallStack::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert!(outcome.reached_put());
        assert_eq!(outcome.stop, RsxAdvanceStop::Reached);
        assert_eq!(
            outcome.methods_dispatched, 2,
            "OFFSET then RELEASE both dispatched across the jump"
        );
        assert_eq!(cursor.get(), put);
        assert_eq!(sem_offset, 0x10);
        assert_eq!(
            emitted.as_slice(),
            &[Effect::RsxLabelWrite {
                offset: 0x10,
                value: 0x99,
            }],
            "RELEASE at the jump target fires after the redirect",
        );
        assert!(call_stack.is_empty(), "Jump does not push a return frame");
    }

    #[test]
    fn advance_stops_on_out_of_range_header_read() {
        let memory = make_memory();
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(0x1_0000); // past the 0x4000-byte region
        cursor.set_put(0x2_0000); // further past
        let mut sem_offset = 0u32;
        let table = NvMethodTable::new();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert_eq!(outcome.stop, RsxAdvanceStop::HeaderOutOfRange);
        assert!(emitted.is_empty());
    }

    #[test]
    fn advance_stops_on_arg_out_of_range() {
        let mut memory = make_memory();
        // Header sits at the last word; its arg would fall off.
        let header_addr = 0x4000 - 4;
        let header = encode_header(NV406E_SEMAPHORE_OFFSET, 1);
        memory
            .apply_commit(
                ByteRange::new(GuestAddr::new(header_addr as u64), 4).unwrap(),
                &header.to_be_bytes(),
            )
            .unwrap();
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(header_addr);
        cursor.set_put(header_addr + 8); // claims one arg word after the header
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert_eq!(outcome.stop, RsxAdvanceStop::ArgOutOfRange);
        assert_eq!(
            cursor.get(),
            header_addr,
            "cursor must still point at the header for re-entry"
        );
        assert!(emitted.is_empty());
    }

    #[test]
    fn advance_stops_on_truncated_method() {
        let mut memory = make_memory();
        let words = [encode_header(NV406E_SEMAPHORE_OFFSET, 3), 0x10u32];
        write_fifo_words(&mut memory, FIFO_BASE, &words);
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(FIFO_BASE);
        cursor.set_put(FIFO_BASE + 8); // one arg, not three
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert_eq!(outcome.stop, RsxAdvanceStop::TruncatedMethod);
        assert_eq!(
            cursor.get(),
            FIFO_BASE,
            "truncated-method stop preserves cursor at the header"
        );
        assert_eq!(
            outcome.methods_dispatched, 0,
            "handler must not fire on a truncated method"
        );
        assert!(emitted.is_empty());
    }

    #[test]
    fn advance_stops_on_address_overflow() {
        // Map at the top of u32; put header at the last 4 bytes so
        // `get + 4` overflows on advance even for zero-count.
        use cellgov_mem::{PageSize, Region};
        let region = Region::new(0xFFFF_0000, 0x1_0000, "overflow_top", PageSize::Page64K);
        let mut memory = GuestMemory::from_regions(vec![region]).unwrap();
        let header_addr: u32 = 0xFFFF_FFFC;
        let header = encode_header(0x100, 0);
        memory
            .apply_commit(
                ByteRange::new(GuestAddr::new(header_addr as u64), 4).unwrap(),
                &header.to_be_bytes(),
            )
            .unwrap();
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(header_addr);
        cursor.set_put(u32::MAX);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert_eq!(
            outcome.stop,
            RsxAdvanceStop::AddressOverflow,
            "checked_add must catch the u32 wrap before any arg read"
        );
        assert_eq!(
            cursor.get(),
            header_addr,
            "overflow stop preserves cursor at the header"
        );
        assert!(emitted.is_empty());
    }

    #[test]
    fn advance_refuses_wrapped_cursor() {
        let memory = make_memory();
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(0x100);
        cursor.set_get(0x200);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert_eq!(outcome.stop, RsxAdvanceStop::WrappedCursor);
        assert_eq!(cursor.get(), 0x200, "cursor state preserved on refusal");
        assert!(emitted.is_empty());
    }

    #[test]
    fn advance_malformed_stop_carries_raw_header() {
        let mut memory = make_memory();
        let bogus = 0x0001_0000u32;
        write_fifo_words(&mut memory, FIFO_BASE, &[bogus]);
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(FIFO_BASE);
        cursor.set_put(FIFO_BASE + 4);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert_eq!(outcome.stop, RsxAdvanceStop::Malformed { raw: bogus });
        assert_eq!(cursor.get(), FIFO_BASE);
    }

    #[test]
    fn advance_non_increment_passes_full_arg_slice_to_single_handler() {
        use std::sync::Mutex;
        static LAST_ARGS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
        fn record_args(_ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
            let mut slot = LAST_ARGS.lock().unwrap();
            slot.clear();
            slot.extend_from_slice(args);
        }
        LAST_ARGS.lock().unwrap().clear();

        let mut memory = make_memory();
        let method: u16 = 0x0400;
        let count: u32 = 2;
        let header = (1u32 << 30) | (count << NV_COUNT_SHIFT) | (method as u32);
        write_fifo_words(&mut memory, FIFO_BASE, &[header, 0x11, 0x22]);
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(FIFO_BASE);
        cursor.set_put(FIFO_BASE + 12);
        let mut sem_offset = 0u32;
        let mut table = NvMethodTable::new();
        table.register_unique(method, record_args).unwrap();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert!(outcome.reached_put());
        assert_eq!(
            outcome.methods_dispatched, 1,
            "NonIncrement dispatches once with full slice"
        );
        assert_eq!(*LAST_ARGS.lock().unwrap(), vec![0x11, 0x22]);
    }

    #[test]
    fn advance_increment_dispatches_per_arg_to_sequential_methods() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CALLS_METHOD_A: AtomicU32 = AtomicU32::new(0);
        static CALLS_METHOD_B: AtomicU32 = AtomicU32::new(0);
        fn handler_a(_ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
            assert_eq!(args, &[0x11]);
            CALLS_METHOD_A.fetch_add(1, Ordering::SeqCst);
        }
        fn handler_b(_ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
            assert_eq!(args, &[0x22]);
            CALLS_METHOD_B.fetch_add(1, Ordering::SeqCst);
        }
        CALLS_METHOD_A.store(0, Ordering::SeqCst);
        CALLS_METHOD_B.store(0, Ordering::SeqCst);

        let mut memory = make_memory();
        let method_a: u16 = 0x0400;
        let method_b: u16 = 0x0404;
        let count: u32 = 2;
        let header = (count << NV_COUNT_SHIFT) | (method_a as u32);
        write_fifo_words(&mut memory, FIFO_BASE, &[header, 0x11, 0x22]);
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(FIFO_BASE);
        cursor.set_put(FIFO_BASE + 12);
        let mut sem_offset = 0u32;
        let mut table = NvMethodTable::new();
        table.register_unique(method_a, handler_a).unwrap();
        table.register_unique(method_b, handler_b).unwrap();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert!(outcome.reached_put());
        assert_eq!(
            outcome.methods_dispatched, 2,
            "Increment dispatches once per arg"
        );
        assert_eq!(CALLS_METHOD_A.load(Ordering::SeqCst), 1);
        assert_eq!(CALLS_METHOD_B.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn advance_unknown_non_increment_header_counts_as_single_attempt() {
        let mut memory = make_memory();
        let unknown: u16 = 0x0300;
        let count: u32 = 3;
        let header = (1u32 << 30) | (count << NV_COUNT_SHIFT) | (unknown as u32);
        write_fifo_words(&mut memory, FIFO_BASE, &[header, 0x11, 0x22, 0x33]);
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(FIFO_BASE);
        cursor.set_put(FIFO_BASE + 16);
        let mut sem_offset = 0u32;
        let table = NvMethodTable::new();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert!(outcome.reached_put());
        assert_eq!(
            outcome.methods_unknown, 1,
            "NonIncrement counts as one attempt even with count=3"
        );
        assert_eq!(outcome.methods_dispatched, 0);
    }

    #[test]
    fn advance_dispatches_set_reference_into_cursor() {
        let mut memory = make_memory();
        let words = [encode_header(NV406E_SET_REFERENCE, 1), 0xABCD_1234u32];
        write_fifo_words(&mut memory, FIFO_BASE, &words);
        let put = FIFO_BASE + (words.len() as u32) * 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(FIFO_BASE);
        cursor.set_put(put);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert!(outcome.reached_put());
        assert_eq!(outcome.methods_dispatched, 1);
        assert_eq!(cursor.current_reference(), 0xABCD_1234);
        assert!(
            emitted.is_empty(),
            "SET_REFERENCE updates state, emits nothing"
        );
    }

    #[test]
    fn advance_handles_zero_count_header() {
        let mut memory = make_memory();
        let words = [
            encode_header(NV406E_SEMAPHORE_OFFSET, 0),
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x40u32,
            encode_header(NV406E_SEMAPHORE_RELEASE, 1),
            0x77u32,
        ];
        write_fifo_words(&mut memory, FIFO_BASE, &words);
        let put = FIFO_BASE + (words.len() as u32) * 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = {
            let mut call_stack = RsxCallStack::new();
            rsx_advance(
                &memory,
                &IoMap::IDENTITY,
                &mut cursor,
                &mut sem_offset,
                &mut call_stack,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            )
        };
        assert!(outcome.reached_put());
        assert_eq!(
            outcome.methods_dispatched, 2,
            "zero-count Increment dispatches zero times"
        );
        assert_eq!(sem_offset, 0x40);
        assert_eq!(
            emitted.as_slice(),
            &[Effect::RsxLabelWrite {
                offset: 0x40,
                value: 0x77,
            }]
        );
    }

    #[test]
    fn return_on_empty_stack_emits_underflow_raw_not_overflow_raw() {
        // Locks the wire-level raw byte the Return arm emits on
        // an empty stack. The pop's typed Err(CallStackUnderflow)
        // and the push's typed Err(CallStackOverflow) cannot
        // unify, but the raw u32 they each map to in the
        // Malformed { raw } can be silently swapped -- this test
        // catches that swap. Single Return header at FIFO_BASE,
        // empty stack, asserts Malformed { raw: 0x4000_02FF }.
        let mut memory = make_memory();
        write_fifo_words(&mut memory, FIFO_BASE, &[NV_FLAG_RETURN]);
        let put = FIFO_BASE + 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let mut call_stack = RsxCallStack::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert!(!outcome.reached_put());
        assert_eq!(
            outcome.stop,
            RsxAdvanceStop::Malformed {
                raw: RSX_ADVANCE_UNDERFLOW_RAW,
            },
            "Return-with-empty-stack must emit RSX_ADVANCE_UNDERFLOW_RAW (0x4000_02FF), \
             NOT CALL_STACK_OVERFLOW_RAW (0x4000_00FF); arm swap would compile clean and \
             silently flip the wire-level fault classification.",
        );
        assert_ne!(
            outcome.stop,
            RsxAdvanceStop::Malformed {
                raw: CALL_STACK_OVERFLOW_RAW,
            },
            "redundant anti-aliasing assert: empty-pop must not surface as overflow",
        );
    }

    #[test]
    fn call_overflow_emits_overflow_raw_not_underflow_raw() {
        // Symmetric to the Return-underflow test. Build a FIFO of
        // CALL_STACK_DEPTH + 1 Call headers each redirecting to
        // itself, so the walker pushes CAP times successfully and
        // the (CAP + 1)th push fails. Asserts the failed-push
        // Malformed carries CALL_STACK_OVERFLOW_RAW, NOT
        // RSX_ADVANCE_UNDERFLOW_RAW. A swap of the two raw
        // constants between push's err-arm and pop's err-arm
        // type-checks cleanly; only this assertion catches it.
        let mut memory = make_memory();
        // Self-pointing Call at FIFO_BASE: each iteration pushes
        // (FIFO_BASE + 4) and resets cursor.get to FIFO_BASE. After
        // CAP successful pushes the (CAP + 1)th overflows.
        let call_header = NV_FLAG_CALL | FIFO_BASE;
        write_fifo_words(&mut memory, FIFO_BASE, &[call_header]);
        let put = FIFO_BASE + 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let mut call_stack = RsxCallStack::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert!(!outcome.reached_put());
        assert_eq!(
            outcome.stop,
            RsxAdvanceStop::Malformed {
                raw: CALL_STACK_OVERFLOW_RAW,
            },
            "Call past stack depth must emit CALL_STACK_OVERFLOW_RAW (0x4000_00FF), \
             NOT RSX_ADVANCE_UNDERFLOW_RAW (0x4000_02FF) or \
             RSX_ADVANCE_ITERATION_CAP_RAW (0x4000_01FF); arm swap would compile clean.",
        );
        assert_ne!(
            outcome.stop,
            RsxAdvanceStop::Malformed {
                raw: RSX_ADVANCE_UNDERFLOW_RAW,
            },
            "redundant anti-aliasing assert: push-overflow must not surface as underflow",
        );
        // Call stack should have CAP successful pushes recorded
        // before the failure; not zero (would mean we tripped
        // before any push) and not CAP + 1 (the failed push must
        // not have mutated state).
        assert_eq!(
            call_stack.depth() as usize,
            CALL_STACK_DEPTH,
            "{CALL_STACK_DEPTH} Call headers pushed before the {}th overflowed",
            CALL_STACK_DEPTH + 1,
        );
    }

    #[test]
    fn jump_target_past_put_surfaces_wrapped_cursor_not_silent_drift() {
        // F1: a Jump that lands `get` past `put` must surface
        // WrappedCursor on the next iteration. Without the in-loop
        // `get > put` check the walker would read whatever bytes
        // live at the target until malformed / unmapped / cap fires
        // -- a silent desynchronization the entry-only check
        // couldn't catch. Layout: Jump at FIFO_BASE pointing at
        // FIFO_BASE + 0x100; put = FIFO_BASE + 4 (one word past the
        // Jump header), so the redirect lands well past put.
        let mut memory = make_memory();
        let jump_target = FIFO_BASE + 0x100;
        let jump_header = NV_FLAG_JUMP | jump_target;
        write_fifo_words(&mut memory, FIFO_BASE, &[jump_header]);
        let put = FIFO_BASE + 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let mut call_stack = RsxCallStack::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert_eq!(
            outcome.stop,
            RsxAdvanceStop::WrappedCursor,
            "post-Jump-redirect get > put must surface WrappedCursor, \
             not silently keep reading until malformed / cap",
        );
        assert_eq!(
            cursor.get(),
            jump_target,
            "cursor parks at the jump target so the caller sees where the wrap occurred",
        );
        assert!(emitted.is_empty());
    }

    #[test]
    fn self_pointing_jump_terminates_at_iteration_cap() {
        // F2: a self-pointing Jump pushes nothing onto the call
        // stack, so the stack-depth cap can never catch it. Only
        // the iteration cap terminates the walk. This test pins
        // that termination: a single Jump at FIFO_BASE whose target
        // is FIFO_BASE itself spins until RSX_ADVANCE_ITERATION_CAP
        // fires. Mirrors the call_overflow arm-swap canary --
        // asserts the cap raw, asserts-ne the two control-flow
        // raws, so a future raw-constant swap is caught.
        //
        // get = put = FIFO_BASE + 4 is intentional: the Jump's
        // target is FIFO_BASE itself (back to the header), so after
        // every redirect get == FIFO_BASE < put. The loop never
        // hits the break condition; only the cap saves it.
        let mut memory = make_memory();
        let self_jump = NV_FLAG_JUMP | FIFO_BASE;
        write_fifo_words(&mut memory, FIFO_BASE, &[self_jump]);
        let put = FIFO_BASE + 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let mut call_stack = RsxCallStack::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert!(!outcome.reached_put());
        assert_eq!(
            outcome.stop,
            RsxAdvanceStop::Malformed {
                raw: RSX_ADVANCE_ITERATION_CAP_RAW,
            },
            "self-pointing Jump must terminate via the iteration cap \
             (0x4000_01FF); a Jump pushes nothing so the stack cap can \
             never fire, and silent spin is the worst outcome here.",
        );
        assert_ne!(
            outcome.stop,
            RsxAdvanceStop::Malformed {
                raw: CALL_STACK_OVERFLOW_RAW,
            },
            "redundant anti-aliasing assert: iteration cap must not surface as call-stack overflow",
        );
        assert_ne!(
            outcome.stop,
            RsxAdvanceStop::Malformed {
                raw: RSX_ADVANCE_UNDERFLOW_RAW,
            },
            "redundant anti-aliasing assert: iteration cap must not surface as call-stack underflow",
        );
        assert!(call_stack.is_empty(), "Jump never pushes a return frame");
    }

    #[test]
    fn advance_honors_new_jump_redirect_and_reaches_put_via_target() {
        // F3: NewJump and Jump arms collapse in rsx_advance, but
        // their headers carry the offset in different bit fields
        // (NV_OLD_JUMP_OFFSET_MASK is 29-bit, NV_NEW_JUMP_OFFSET_MASK
        // is 30-bit). decode_header normalizes them; this integration
        // test pins that the rsx_advance + decode_header pipeline
        // correctly extracts the NewJump target. Mirrors the existing
        // Jump redirect test layout.
        use crate::rsx::method::NV_FLAG_NEW_JUMP;
        let mut memory = make_memory();
        let jump_target = FIFO_BASE + 0x20;
        let jump_header = NV_FLAG_NEW_JUMP | jump_target;
        let prologue = [
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x10u32,
            jump_header,
        ];
        let target_block = [encode_header(NV406E_SEMAPHORE_RELEASE, 1), 0x99u32];
        write_fifo_words(&mut memory, FIFO_BASE, &prologue);
        write_fifo_words(&mut memory, jump_target, &target_block);
        let put = jump_target + (target_block.len() as u32) * 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let mut call_stack = RsxCallStack::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert!(outcome.reached_put());
        assert_eq!(outcome.methods_dispatched, 2);
        assert_eq!(cursor.get(), put);
        assert_eq!(sem_offset, 0x10);
        assert_eq!(
            emitted.as_slice(),
            &[Effect::RsxLabelWrite {
                offset: 0x10,
                value: 0x99,
            }],
        );
        assert!(
            call_stack.is_empty(),
            "NewJump does not push a return frame"
        );
    }

    #[test]
    fn partial_increment_dispatches_known_and_counts_unknown_per_sub_method() {
        // F4: an Increment with N args dispatches each sub-method
        // (method+0, method+4, ..., method+4*(N-1)) independently.
        // The mixed case -- method_a registered, method_a+4 not --
        // must produce (dispatched=1, unknown=1). Without this test
        // a future decoder change that mis-shifts the sub-method
        // computation could silently flip the dispatched/unknown
        // split.
        //
        // Asserting which arg the dispatched handler received
        // (args == &[0x11], the i=0 arg) is what makes this
        // self-contained: a swap that dispatched method_a for i=1
        // and treated method_a as unknown for i=0 would still
        // produce (dispatched=1, unknown=1) and would pass without
        // this check.
        use std::sync::Mutex;
        static SEEN_ARGS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
        fn record(_ctx: &mut NvDispatchContext<'_>, args: &[u32]) {
            let mut slot = SEEN_ARGS.lock().unwrap();
            slot.clear();
            slot.extend_from_slice(args);
        }
        SEEN_ARGS.lock().unwrap().clear();

        let mut memory = make_memory();
        let method_a: u16 = 0x0500;
        let count: u32 = 2;
        let header = (count << NV_COUNT_SHIFT) | (method_a as u32);
        write_fifo_words(&mut memory, FIFO_BASE, &[header, 0x11, 0x22]);
        let put = FIFO_BASE + 12;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(FIFO_BASE);
        cursor.set_put(put);
        let mut sem_offset = 0u32;
        let mut table = NvMethodTable::new();
        table.register_unique(method_a, record).unwrap();
        // method_a + 4 (= 0x0504) intentionally NOT registered.
        let mut emitted: Vec<Effect> = Vec::new();
        let mut call_stack = RsxCallStack::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert!(outcome.reached_put());
        assert_eq!(
            outcome.methods_dispatched, 1,
            "method_a fires once; method_a+4 has no handler",
        );
        assert_eq!(
            outcome.methods_unknown, 1,
            "unknown count is per-sub-method, not per-header, for Increment",
        );
        assert_eq!(
            *SEEN_ARGS.lock().unwrap(),
            vec![0x11],
            "method_a must receive the i=0 arg (0x11); seeing 0x22 instead would mean \
             the sub-method address computation is off by one and method_a was \
             dispatched for i=1 while method_a+0 silently went to unknown",
        );
        assert_eq!(cursor.get(), put);
    }

    #[test]
    fn unpaired_release_emits_label_write_at_offset_zero() {
        // F6: NV406E_SEMAPHORE_RELEASE without a preceding
        // NV406E_SEMAPHORE_OFFSET uses whatever sem_offset holds.
        // For a fresh drain that value is 0; the handler doc
        // (method.rs nv406e_semaphore_release) explicitly accepts
        // this as the CPU-visible outcome of a guest-side bug.
        //
        // The authoritative policy lives in method.rs; this test is
        // the integration-level corroboration that rsx_advance
        // plumbs sem_offset through without imposing its own
        // filtering. If the handler policy ever flips to a loud
        // fault, the fix lands in method.rs and this test must be
        // replaced with one that asserts the new fault path.
        let mut memory = make_memory();
        let words = [encode_header(NV406E_SEMAPHORE_RELEASE, 1), 0xFEEDu32];
        write_fifo_words(&mut memory, FIFO_BASE, &words);
        let put = FIFO_BASE + (words.len() as u32) * 4;
        let mut cursor = RsxFifoCursor::new();
        cursor.set_put(put);
        cursor.set_get(FIFO_BASE);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let mut call_stack = RsxCallStack::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert!(outcome.reached_put());
        assert_eq!(outcome.methods_dispatched, 1);
        assert_eq!(
            emitted.as_slice(),
            &[Effect::RsxLabelWrite {
                offset: 0,
                value: 0xFEED,
            }],
            "unpaired RELEASE uses sem_offset=0; cross-drain carry is intentional \
             per the handler doc, NOT a silent state-machine error",
        );
    }
}
