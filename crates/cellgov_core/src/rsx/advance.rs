//! Commit-boundary FIFO advance pass.
//!
//! Drains from `cursor.get` up to `cursor.put`, dispatching through
//! the [`NvMethodTable`] and pushing emissions into a caller-owned
//! `Vec<Effect>`. The caller forwards them into batch N+1; the
//! advance pass cannot observe its own emissions within the same
//! batch.
//!
//! Contract summary (details on [`rsx_advance`]):
//!
//! - Requires `cursor.get <= cursor.put` at entry; `get > put`
//!   returns [`RsxAdvanceStop::WrappedCursor`] without mutating.
//! - Control-flow and Malformed headers stop the drain cleanly
//!   with the cursor parked on the offending header.
//! - Unknown methods advance past their args and continue.

use crate::rsx::method::{decode_header, NvCommandKind, NvDispatchContext, NvMethodTable};
use crate::rsx::RsxFifoCursor;
use cellgov_effects::Effect;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::GuestTicks;

/// Terminal condition of a single advance-pass call.
///
/// Every non-`Reached` variant parks `cursor.get` at the offending
/// header (or leaves it unchanged for the entry-condition
/// failures), so a caller can re-enter safely once the condition
/// is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsxAdvanceStop {
    /// Drain reached `cursor.put` cleanly.
    Reached,
    /// Stopped at a control-flow header; control-flow dispatch is not implemented.
    ControlFlow,
    /// Stopped at a header the decoder classified as malformed.
    Malformed {
        /// The raw 32-bit header word.
        raw: u32,
    },
    /// Header read at `cursor.get` fell in unmapped memory; `get` unchanged.
    HeaderOutOfRange,
    /// An argument-word read fell in unmapped memory; `get` parked at the header.
    ArgOutOfRange,
    /// Declared arg count would advance past `cursor.put`.
    TruncatedMethod,
    /// `get + header + args` would overflow u32.
    AddressOverflow,
    /// `cursor.get > cursor.put` at entry; wrap-aware draining is not implemented.
    WrappedCursor,
}

/// Summary of one advance-pass invocation. Counters saturate at `u32::MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxAdvanceOutcome {
    /// Dispatches that hit a registered handler.
    pub methods_dispatched: u32,
    /// Dispatch attempts that fell through to the unknown-method fallback.
    ///
    /// Counted in **dispatch attempts**, not headers: an Increment
    /// header with `count = N` and no matching handler contributes
    /// `N`; a NonIncrement header contributes `1` regardless of
    /// `count`.
    pub methods_unknown: u32,
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

fn read_fifo_word(memory: &GuestMemory, addr: u32) -> Option<u32> {
    let range = ByteRange::new(GuestAddr::new(addr as u64), 4)?;
    let bytes = memory.read(range)?;
    let word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    Some(word)
}

/// Drain the FIFO from `cursor.get()` up to `cursor.put()`.
///
/// Advances `cursor.get`, updates `sem_offset` via the semaphore
/// handlers, and appends emissions to `emitted` in FIFO address
/// order. FIFO bytes are interpreted as little-endian u32.
///
/// Entry requires `cursor.get() <= cursor.put()`. `get == put`
/// returns immediately with `stop = Reached`; `get > put` returns
/// [`RsxAdvanceStop::WrappedCursor`] without mutation.
///
/// Increment headers dispatch once per arg against `method + 4*i`.
/// NonIncrement headers dispatch once with the full slice.
/// Zero-count NonIncrement still dispatches once with an empty
/// slice, so every handler must index via `args.first()` / `get`;
/// direct indexing will panic.
pub fn rsx_advance(
    memory: &GuestMemory,
    cursor: &mut RsxFifoCursor,
    sem_offset: &mut u32,
    table: &NvMethodTable,
    emitted: &mut Vec<Effect>,
    now: GuestTicks,
) -> RsxAdvanceOutcome {
    let mut outcome = RsxAdvanceOutcome {
        methods_dispatched: 0,
        methods_unknown: 0,
        stop: RsxAdvanceStop::Reached,
    };
    if cursor.get() == cursor.put() {
        return outcome;
    }
    if cursor.get() > cursor.put() {
        outcome.stop = RsxAdvanceStop::WrappedCursor;
        return outcome;
    }
    while cursor.get() < cursor.put() {
        let get = cursor.get();
        let header_word = match read_fifo_word(memory, get) {
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
                // wrapping_add below is equivalent to `+` here: the
                // checked_add/checked_mul above proved next_get fits
                // in u32. Using wrapping_add means a future refactor
                // that reorders the bounds check fails the nearest
                // overflow test rather than reading low memory.
                let args_start = get.wrapping_add(4);
                let mut args: Vec<u32> = Vec::with_capacity(count as usize);
                for i in 0..count {
                    let arg_addr = args_start.wrapping_add(i.wrapping_mul(4));
                    match read_fifo_word(memory, arg_addr) {
                        Some(w) => args.push(w),
                        None => {
                            outcome.stop = RsxAdvanceStop::ArgOutOfRange;
                            return outcome;
                        }
                    }
                }
                cursor.set_get(next_get);
                match header.kind {
                    NvCommandKind::Increment => {
                        for (i, arg) in args.iter().enumerate() {
                            let sub_method = header.method.wrapping_add((i as u16) * 4);
                            match table.lookup(sub_method) {
                                Some(handler) => {
                                    let mut ctx = NvDispatchContext {
                                        cursor,
                                        sem_offset,
                                        emitted,
                                        now,
                                    };
                                    handler(&mut ctx, std::slice::from_ref(arg));
                                    outcome.methods_dispatched =
                                        outcome.methods_dispatched.saturating_add(1);
                                }
                                None => {
                                    outcome.methods_unknown =
                                        outcome.methods_unknown.saturating_add(1);
                                }
                            }
                        }
                    }
                    NvCommandKind::NonIncrement => match table.lookup(header.method) {
                        Some(handler) => {
                            let mut ctx = NvDispatchContext {
                                cursor,
                                sem_offset,
                                emitted,
                                now,
                            };
                            handler(&mut ctx, &args);
                            outcome.methods_dispatched =
                                outcome.methods_dispatched.saturating_add(1);
                        }
                        None => {
                            outcome.methods_unknown = outcome.methods_unknown.saturating_add(1);
                        }
                    },
                    _ => unreachable!("outer match guards Increment / NonIncrement only"),
                }
            }
            NvCommandKind::Jump { .. }
            | NvCommandKind::NewJump { .. }
            | NvCommandKind::Call { .. }
            | NvCommandKind::Return => {
                outcome.stop = RsxAdvanceStop::ControlFlow;
                return outcome;
            }
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
    use crate::rsx::method::{
        register_nv406e_label_handlers, register_nv406e_reference_handler, NV406E_SEMAPHORE_OFFSET,
        NV406E_SEMAPHORE_RELEASE, NV406E_SET_REFERENCE, NV_COUNT_SHIFT, NV_FLAG_JUMP,
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
            bytes.extend_from_slice(&w.to_le_bytes());
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert_eq!(
            outcome,
            RsxAdvanceOutcome {
                methods_dispatched: 0,
                methods_unknown: 0,
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

        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
            rsx_advance(
                &memory,
                &mut cursor,
                &mut sem_offset,
                &table,
                &mut emitted,
                GuestTicks::ZERO,
            );
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
        rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
    fn advance_stops_cleanly_on_control_flow_header() {
        let mut memory = make_memory();
        let jump_offset = 0x0000_0100u32;
        let jump_header = NV_FLAG_JUMP | jump_offset;
        let words = [
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x10u32,
            jump_header,
            // RELEASE follows the jump; the drain must stop before
            // it fires, not mis-read the offset as args.
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        assert!(!outcome.reached_put());
        assert_eq!(outcome.stop, RsxAdvanceStop::ControlFlow);
        assert_eq!(outcome.methods_dispatched, 1);
        assert_eq!(
            cursor.get(),
            FIFO_BASE + 8,
            "cursor parks AT the jump header, not past it"
        );
        assert_eq!(sem_offset, 0x10);
        assert!(
            emitted.is_empty(),
            "only OFFSET (no effect) fired; RELEASE did not reach"
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
                &header.to_le_bytes(),
            )
            .unwrap();
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(header_addr);
        cursor.set_put(header_addr + 8); // claims one arg word after the header
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
                &header.to_le_bytes(),
            )
            .unwrap();
        let mut cursor = RsxFifoCursor::new();
        cursor.set_get(header_addr);
        cursor.set_put(u32::MAX);
        let mut sem_offset = 0u32;
        let table = setup_table();
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
        let outcome = rsx_advance(
            &memory,
            &mut cursor,
            &mut sem_offset,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
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
}
