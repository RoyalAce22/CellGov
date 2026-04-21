//! RSX FIFO advance pass.
//!
//! Drains the guest's FIFO command buffer from `cursor.get` up to
//! `cursor.put` by decoding each 32-bit command header, consuming the
//! declared argument words, and dispatching any registered handler in
//! the [`NvMethodTable`]. Emissions are pushed into a caller-provided
//! `Vec<Effect>`; the caller forwards them to the commit pipeline on
//! the next batch. The atomic-batch contract means the advance pass
//! cannot observe its own emissions within the same batch.
//!
//! Scope boundaries:
//!
//! - Normal methods (Increment / NonIncrement) dispatch through the
//!   table. The count field is honored; each argument word lives in
//!   FIFO memory immediately after the header.
//! - Control-flow commands (Jump / NewJump / Call / Return) stop the
//!   drain cleanly without desync. A production drain has to handle
//!   them; stopping is a safer default than mis-interpreting the
//!   offset as a method address until control-flow handling lands.
//! - Malformed headers stop the drain cleanly. The caller learns via
//!   the returned [`RsxAdvanceOutcome`] whether the drain reached
//!   `put` or stopped early.
//! - Unknown methods (those with no handler in the table) advance
//!   past their argument count and continue -- the documented
//!   unknown-method fallback.

use crate::rsx::RsxFifoCursor;
use crate::rsx_method::{decode_header, NvCommandKind, NvDispatchContext, NvMethodTable};
use cellgov_effects::Effect;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::GuestTicks;

/// Reason the drain stopped. `Reached` means the drain consumed the
/// FIFO cleanly up to `cursor.put`; every other variant marks an
/// early exit with a specific diagnostic. The carried data is
/// deliberately narrow -- just enough for a caller to produce a
/// meaningful error message without re-reading memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsxAdvanceStop {
    /// Drain consumed the FIFO and `cursor.get == cursor.put`.
    Reached,
    /// Drain stopped at a control-flow header (Jump / NewJump / Call
    /// / Return). `cursor.get` still points at the unhandled header
    /// so a future control-flow implementation can pick up where
    /// this drain parked. Control-flow handling is currently
    /// deferred.
    ControlFlow,
    /// Drain stopped at a malformed header. The raw header word is
    /// captured so diagnostics can name the offending bit pattern
    /// without re-reading memory. `cursor.get` still points at the
    /// malformed header.
    Malformed {
        /// The raw 32-bit header word the decoder classified as
        /// malformed.
        raw: u32,
    },
    /// Drain stopped because the header-word read at `cursor.get`
    /// went out of bounds -- the cursor itself points at unmapped
    /// memory. `cursor.get` is unchanged.
    HeaderOutOfRange,
    /// Drain stopped because an argument-word read went out of
    /// bounds partway through decoding a method -- the header was
    /// valid and read successfully, but one of its arg slots lies
    /// in unmapped memory. `cursor.get` still points at the header
    /// (re-entry safe).
    ArgOutOfRange,
    /// Drain stopped because the current header's declared arg
    /// count would advance `cursor.get` past `cursor.put` -- the
    /// FIFO stream is truncated or its count field is wrong.
    /// `cursor.get` still points at the header.
    TruncatedMethod,
    /// Drain stopped because the 32-bit address arithmetic would
    /// wrap around the u32 range. Only triggered by a pathological
    /// put / get value or a count that crosses the 4 GiB boundary.
    /// `cursor.get` still points at the header.
    AddressOverflow,
    /// Drain refused to run because `cursor.get > cursor.put` at
    /// entry and the cursor has no ring-size knowledge to perform a
    /// wrap-aware drain. `cursor.get` is unchanged. Ring-aware
    /// draining is a later-slice concern.
    WrappedCursor,
}

/// Summary of one advance-pass invocation.
///
/// Callers use `stop` to distinguish the drain's terminal condition
/// and the counters to assert handler firing. The counters are
/// saturating: a pathological FIFO with more than `u32::MAX`
/// methods caps the count rather than wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxAdvanceOutcome {
    /// Number of normal-method headers the drain dispatched through
    /// a registered handler.
    pub methods_dispatched: u32,
    /// Number of dispatch attempts whose method address had no
    /// handler registered (unknown-method fallback path). The drain
    /// still advances past their arguments and keeps going.
    ///
    /// The unit is **dispatch attempts**, not headers. An Increment
    /// header with `count = N` and no handler at any of
    /// `method`, `method + 4`, ..., `method + 4*(N-1)` contributes
    /// `N` to this count (one per per-arg dispatch). A NonIncrement
    /// header with no handler at `method` contributes `1` regardless
    /// of its count. Callers using this for log dedup or rate
    /// limiting should be aware that a single malformed upstream
    /// FIFO writer can inflate this counter without emitting
    /// `N` distinct header addresses.
    pub methods_unknown: u32,
    /// Terminal condition.
    pub stop: RsxAdvanceStop,
}

impl RsxAdvanceOutcome {
    /// Whether the drain consumed the FIFO cleanly up to
    /// `cursor.put`. Convenience over `stop == RsxAdvanceStop::Reached`.
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
/// Mutates `cursor` (advancing `get` past consumed words), mutates
/// `sem_offset` (via the NV406E semaphore offset handler), and
/// appends emitted effects to `emitted`. Returns a summary
/// describing the drain's outcome.
///
/// **FIFO byte order.** PS3 RSX FIFO memory is little-endian u32.
/// The PPU is big-endian and libgcm byte-swaps words before storing
/// them; the bytes in guest memory are the RSX's LE view. This
/// function reads raw bytes and interprets them as `u32::from_le_bytes`.
///
/// **Linear-only.** This slice assumes `cursor.get <= cursor.put` on
/// entry (linear drain). A `get > put` entry sets
/// [`RsxAdvanceStop::WrappedCursor`] and does not attempt ring-aware
/// draining -- the cursor has no ring-size knowledge and the drain
/// cannot reconstruct it. Ring-aware drain is a later slice.
///
/// **Increment vs NonIncrement.** Increment methods dispatch each
/// argument individually against `method + 4*i` (auto-incrementing
/// method address). NonIncrement methods dispatch the full
/// argument slice once against `method`. For count=1, the two are
/// indistinguishable -- which covers every single-argument
/// NV406E / NV4097 handler in this module.
///
/// **Zero-count headers.** Increment with count=0 iterates zero
/// times and dispatches nothing -- a header-only nop. NonIncrement
/// with count=0 still dispatches once, passing an empty slice. The
/// advance pass makes NO claim that a handler expects at least one
/// argument; handlers must defend against empty-slice dispatch via
/// `args.first()` / `args.get(i)` or similar. The registered
/// handlers all defend this way. Handlers that index directly into
/// `args` without a length check will panic on a legitimate zero-
/// count NonIncrement stream.
///
/// **Short-circuit.** Returns immediately with `stop = Reached` if
/// `cursor.get() == cursor.put()`.
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
                // Address arithmetic: args_start = get + 4;
                // next_get = get + 4 + count * 4. Use checked_add
                // / checked_mul so a pathological put / get near
                // u32::MAX cannot wrap silently into low memory.
                let Some(total_bytes_u32) = count.checked_mul(4).and_then(|b| b.checked_add(4))
                else {
                    outcome.stop = RsxAdvanceStop::AddressOverflow;
                    return outcome;
                };
                let Some(next_get) = get.checked_add(total_bytes_u32) else {
                    outcome.stop = RsxAdvanceStop::AddressOverflow;
                    return outcome;
                };
                // Bounds check vs put: the method's header + all
                // args must fit within the linear drain window.
                // Without this a bogus count field silently reads
                // past put into whatever happens to be in memory.
                if next_get > cursor.put() {
                    outcome.stop = RsxAdvanceStop::TruncatedMethod;
                    return outcome;
                }
                // SAFETY (arithmetic, not memory): the checked_add /
                // checked_mul above proved that
                // `get + 4 + count * 4` fits in a u32 without
                // overflow. Every address derived below is
                // `<= next_get` and therefore also fits. The
                // `wrapping_add` here is equivalent to `+` for
                // these specific inputs; it is written as
                // wrapping_add so a future refactor that reorders
                // the bounds check against these adds sees a
                // silent wrap instead of a compile-time-lost
                // invariant and fails the nearest overflow test
                // rather than reading low memory.
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
                        // Each arg targets a distinct method address
                        // (method, method + 4, method + 8, ...). This
                        // matters for multi-arg Increment methods;
                        // for the registered single-arg handlers it
                        // is indistinguishable from NonIncrement.
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
                    NvCommandKind::NonIncrement => {
                        // All args target the same method address,
                        // handler invoked once with the full slice.
                        match table.lookup(header.method) {
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
                        }
                    }
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
    use crate::rsx_method::{
        register_nv406e_label_handlers, register_nv406e_reference_handler, NV406E_SEMAPHORE_OFFSET,
        NV406E_SEMAPHORE_RELEASE, NV406E_SET_REFERENCE, NV_COUNT_SHIFT, NV_FLAG_JUMP,
    };
    use cellgov_mem::GuestMemory;

    const FIFO_BASE: u32 = 0x1000;

    fn make_memory() -> GuestMemory {
        GuestMemory::new(0x4000)
    }

    /// Encode a normal (incrementing) method header with the given
    /// 14-bit method address and 11-bit argument count.
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
        // Two-method sequence: OFFSET=0x80, RELEASE=0x1234_5678.
        // Each method is a single header + one argument word.
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
        // Same scripted FIFO input must produce byte-identical
        // cursor state, sem_offset, and emitted effects across two
        // independent runs. This is the per-slice determinism gate.
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
        // Two RELEASE methods at different offsets must produce
        // label writes in the same order the FIFO stream presents
        // them. No reordering, no deduplication.
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
        // Unknown-method fallback: the drain advances past the
        // argument count and continues, so a KNOWN method after an
        // unknown method still fires. This pins the count-field
        // decode for the advance arithmetic.
        let mut memory = make_memory();
        // Unknown Increment method at address 0x0200 with two args.
        // Under Increment semantics each arg targets a sequential
        // method address (0x200, 0x204), so a 2-arg unknown header
        // counts as two unknown dispatches. Followed by a KNOWN
        // OFFSET + RELEASE pair.
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
        // An old-JUMP header in the stream stops the drain at the
        // header word (cursor.get still points at the jump). No
        // effect is emitted, no method is counted, and the drain
        // does NOT desync by mis-reading the jump offset as args.
        let mut memory = make_memory();
        let jump_offset = 0x0000_0100u32; // bits 2..=28
        let jump_header = NV_FLAG_JUMP | jump_offset;
        let words = [
            encode_header(NV406E_SEMAPHORE_OFFSET, 1),
            0x10u32,
            jump_header,
            // This RELEASE must NOT fire because the drain stops
            // on the jump.
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
        // cursor.put and cursor.get both point past the end of
        // guest memory. The drain must stop cleanly at the first
        // failed header read rather than panicking.
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
        // Header reads successfully at get, but one of its args
        // lies in unmapped memory. Drain must surface ArgOutOfRange
        // (distinct from HeaderOutOfRange) and leave cursor.get at
        // the header for re-entry safety.
        let mut memory = make_memory();
        // Place the header at the last word of the region so the
        // arg word falls off the end.
        let header_addr = 0x4000 - 4; // last 4 bytes of 0x4000-byte region
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
        // The header claims count=3 but put sits only one arg past
        // the header. Without the bounds check the drain would
        // read past put into whatever lies beyond; the check must
        // surface TruncatedMethod instead.
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
        // Drive the checked_add guard end-to-end: map a region at
        // the top of the u32 address space, write a valid header
        // at the last 4 bytes, and set put past u32::MAX's
        // neighborhood. With get = 0xFFFFFFFC, the header reads
        // successfully (mapped) but `get + total_bytes` overflows
        // u32 for ANY non-zero total -- including total = 4 (the
        // bare header plus no args). Without the checked_add the
        // drain would silently wrap into low memory.
        use cellgov_mem::{PageSize, Region};
        let region = Region::new(0xFFFF_0000, 0x1_0000, "overflow_top", PageSize::Page64K);
        let mut memory = GuestMemory::from_regions(vec![region]).unwrap();
        // Zero-count header so the guard fires on `get + 4`, not on
        // `count * 4`. Methods address is arbitrary.
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
        // put is strictly greater than get so the drain enters the
        // loop; any value > get suffices.
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
        // get > put at entry. The cursor has no ring-size knowledge
        // so a wrap-aware drain is not possible; refuse rather than
        // silently mis-drain.
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
        // A header with bit 16 set but no valid method / control-
        // flow classification is malformed per the NV2A decoder.
        // The stop variant must carry the raw word for diagnostics.
        let mut memory = make_memory();
        let bogus = 0x0001_0000u32; // bit 16 set alone
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
        // A NonIncrement header with count=2 targets `method` once
        // with both args in a single invocation. Built-in handlers
        // all have count=1 so this test uses a test-local handler
        // that records the full arg slice it received.
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
        // NonIncrement flag is bit 30.
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
        // An Increment header with count=2 targets `method` for
        // args[0] and `method + 4` for args[1]. Confirm by
        // registering DIFFERENT handlers at both addresses and
        // observing each fired exactly once.
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
        // A NonIncrement header with count=3 and no registered
        // handler contributes exactly one to methods_unknown,
        // because NonIncrement dispatches once regardless of count.
        // This pins the "dispatch attempts, not headers" contract
        // alongside the Increment test (which counts per-arg).
        let mut memory = make_memory();
        let unknown: u16 = 0x0300;
        let count: u32 = 3;
        // NonIncrement flag is bit 30.
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
        // A SET_REFERENCE method drained through the advance pass
        // must land in cursor.current_reference (visible to the
        // guest via the RSX control register's reference slot).
        // No effect emitted; the reference slot is pure RSX state
        // folded into the runtime sync-state hash.
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
        // A method header with count=0 is legal (some NV methods
        // take no args; the header itself is the whole command).
        // The drain must advance past JUST the header word. Under
        // Increment semantics a count=0 header iterates zero times
        // and dispatches nothing; the drain continues past it.
        let mut memory = make_memory();
        // Zero-count OFFSET header (Increment; zero dispatches),
        // then a normal OFFSET+RELEASE pair. The normal pair
        // produces the label write.
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
