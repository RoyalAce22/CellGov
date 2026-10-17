//! Each RSX command header form, held against the reference console.
//!
//! The `rsx_fifo_forms` probe (`tests/micro/rsx_fifo_forms/ppu/main.c`)
//! writes command words by hand into libgcm's command buffer, one step
//! at a time, and polls the reference register or a label after each
//! flush. This file lays the same words into guest memory, drains each
//! step with `rsx_advance` and the default handler table, and compares
//! what CellGov leaves behind with what the console read back.

use super::*;
use crate::rsx::method::{
    back_end_semaphore_value_swap, NV406E_SEMAPHORE_ACQUIRE, NV406E_SEMAPHORE_OFFSET,
    NV406E_SEMAPHORE_RELEASE, NV406E_SET_REFERENCE, NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE,
    NV4097_NO_OPERATION, NV4097_SET_SEMAPHORE_OFFSET, NV_COUNT_SHIFT, NV_FLAG_CALL, NV_FLAG_JUMP,
    NV_FLAG_NEW_JUMP, NV_FLAG_NON_INCREMENT, NV_FLAG_RETURN,
};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::rsx::LABEL_STRIDE;

const FIFO_BASE: u32 = 0x1000;
const LABEL_A: u32 = 65;
const LABEL_B: u32 = 66;

/// The probe's payload, as big-endian u32 words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/rsx_fifo_forms/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

fn hdr(method: u16, count: u32) -> u32 {
    (count << NV_COUNT_SHIFT) | method as u32
}

/// The command buffer as the probe fills it: `here` is the address of
/// the next word, and the IO offset equals it under the identity map.
struct Stream {
    words: Vec<u32>,
    step_ends: Vec<u32>,
}

impl Stream {
    fn here(&self) -> u32 {
        FIFO_BASE + 4 * self.words.len() as u32
    }

    fn emit(&mut self, word: u32) {
        self.words.push(word);
    }

    /// The probe's `rsxFlushBuffer`: the put pointer moves to `here`.
    fn flush(&mut self) {
        let end = self.here();
        self.step_ends.push(end);
    }
}

/// The probe's eight steps, word for word.
fn probe_stream() -> Stream {
    let mut s = Stream {
        words: Vec::new(),
        step_ends: Vec::new(),
    };

    // 0: a method header.
    s.emit(hdr(NV406E_SET_REFERENCE, 1));
    s.emit(0x1001);
    s.flush();

    // 1: non-increment, three arguments to one method.
    s.emit(NV_FLAG_NON_INCREMENT | hdr(NV406E_SET_REFERENCE, 3));
    s.emit(0x2001);
    s.emit(0x2002);
    s.emit(0x2003);
    s.flush();

    // 2: old jump over a SET_REFERENCE.
    let p = s.here();
    s.emit(NV_FLAG_JUMP | (p + 12));
    s.emit(hdr(NV406E_SET_REFERENCE, 1));
    s.emit(0xBAD3);
    s.emit(hdr(NV406E_SET_REFERENCE, 1));
    s.emit(0x3003);
    s.flush();

    // 3: new jump over a SET_REFERENCE.
    let p = s.here();
    s.emit((p + 12) | NV_FLAG_NEW_JUMP);
    s.emit(hdr(NV406E_SET_REFERENCE, 1));
    s.emit(0xBAD4);
    s.emit(hdr(NV406E_SET_REFERENCE, 1));
    s.emit(0x4004);
    s.flush();

    // 4: call a label release, return, set the reference, then jump
    // past the subroutine.
    let p = s.here();
    s.emit((p + 16) | NV_FLAG_CALL);
    s.emit(hdr(NV406E_SET_REFERENCE, 1));
    s.emit(0x5005);
    s.emit(NV_FLAG_JUMP | (p + 36));
    s.emit(hdr(NV406E_SEMAPHORE_OFFSET, 1));
    s.emit(LABEL_A * LABEL_STRIDE);
    s.emit(hdr(NV406E_SEMAPHORE_RELEASE, 1));
    s.emit(0xA5A5);
    s.emit(NV_FLAG_RETURN);
    s.flush();

    // 5: acquire label 65 at the value it holds, then set the reference.
    s.emit(hdr(NV406E_SEMAPHORE_OFFSET, 1));
    s.emit(LABEL_A * LABEL_STRIDE);
    s.emit(hdr(NV406E_SEMAPHORE_ACQUIRE, 1));
    s.emit(0xA5A5);
    s.emit(hdr(NV406E_SET_REFERENCE, 1));
    s.emit(0x6006);
    s.flush();

    // 6: a no-op, then set the reference.
    s.emit(hdr(NV4097_NO_OPERATION, 1));
    s.emit(0);
    s.emit(hdr(NV406E_SET_REFERENCE, 1));
    s.emit(0x7007);
    s.flush();

    // 7: a back-end release into label 66.
    s.emit(hdr(NV4097_SET_SEMAPHORE_OFFSET, 1));
    s.emit(LABEL_B * LABEL_STRIDE);
    s.emit(hdr(NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE, 1));
    s.emit(0x1122_3344);
    s.flush();

    s
}

/// What CellGov holds after each step's drain.
struct StepResult {
    outcome: RsxAdvanceOutcome,
    reference: u32,
    label_writes: Vec<(u32, u32)>,
}

fn drain_steps(stream: &Stream) -> Vec<StepResult> {
    let mut memory = GuestMemory::new(0x4000);
    let len = (stream.words.len() as u64) * 4;
    let range = ByteRange::new(GuestAddr::new(FIFO_BASE as u64), len).unwrap();
    let bytes: Vec<u8> = stream.words.iter().flat_map(|w| w.to_be_bytes()).collect();
    memory.apply_commit(range, &bytes).unwrap();

    let table = NvMethodTable::with_default_handlers();
    let mut cursor = RsxFifoCursor::new();
    cursor.set_get(FIFO_BASE);
    let mut sem_offset = 0u32;
    let mut call_stack = RsxCallStack::new();

    let mut results = Vec::new();
    for &end in &stream.step_ends {
        cursor.set_put(end);
        let mut emitted: Vec<Effect> = Vec::new();
        let outcome = rsx_advance(
            &memory,
            &IoMap::IDENTITY,
            &mut cursor,
            &mut sem_offset,
            0,
            &mut call_stack,
            &table,
            &mut emitted,
            GuestTicks::ZERO,
        );
        let label_writes = emitted
            .iter()
            .filter_map(|e| match e {
                Effect::RsxLabelWrite { offset, value } => Some((*offset, *value)),
                _ => None,
            })
            .collect();
        results.push(StepResult {
            outcome,
            reference: cursor.current_reference(),
            label_writes,
        });
    }
    results
}

#[test]
fn every_step_the_console_reached_drains_to_put() {
    let words = console_words();
    assert_eq!(words[0], 0, "rsxInit");
    assert_eq!(words[2], 8, "the console reached all eight steps");
    let results = drain_steps(&probe_stream());
    assert_eq!(results.len(), 8);
    for (k, r) in results.iter().enumerate() {
        assert_eq!(words[3 + 3 * k], 0, "step {k} seen on the console");
        assert_eq!(r.outcome.stop, RsxAdvanceStop::Reached, "step {k}");
    }
}

#[test]
fn the_reference_after_each_step_is_the_word_the_console_polled() {
    // Steps 0-6 poll the reference register; step 7 polls label 66.
    let words = console_words();
    let results = drain_steps(&probe_stream());
    for (k, r) in results.iter().enumerate().take(7) {
        assert_eq!(r.reference, words[4 + 3 * k], "step {k}");
    }
}

#[test]
fn the_label_stride_is_the_one_libgcm_spaces_labels_by() {
    let words = console_words();
    assert_eq!(LABEL_STRIDE, words[1]);
}

#[test]
fn the_called_release_writes_label_65_as_on_the_console() {
    let words = console_words();
    let results = drain_steps(&probe_stream());
    assert_eq!(
        results[4].label_writes,
        vec![(LABEL_A * LABEL_STRIDE, words[27])]
    );
}

#[test]
fn the_back_end_release_lands_byte_swapped_in_label_66_as_on_the_console() {
    // The console read 0x11223344 back from label 66 with bytes 0 and 2
    // exchanged.
    let words = console_words();
    let results = drain_steps(&probe_stream());
    assert_eq!(words[24], 0, "label 66 changed on the console");
    assert_eq!(
        results[7].label_writes,
        vec![(LABEL_B * LABEL_STRIDE, words[25])]
    );
    assert_eq!(back_end_semaphore_value_swap(0x1122_3344), words[25]);
}

#[test]
fn the_acquire_and_the_no_op_take_the_unknown_method_fallback() {
    // The console's acquire found label 65 already at its value, so the
    // capture shows only that the stream went on past it. CellGov
    // registers no handler for the acquire or the no-op; each counts one
    // unknown method and the drain goes on, which the reference words
    // above match.
    let results = drain_steps(&probe_stream());
    assert_eq!(results[5].outcome.methods_unknown, 1);
    assert_eq!(results[6].outcome.methods_unknown, 1);
    for (k, r) in results.iter().enumerate() {
        if k != 5 && k != 6 {
            assert_eq!(r.outcome.methods_unknown, 0, "step {k}");
        }
    }
}
