//! FIFO decode held against console-captured RSX output.
//!
//! Most assertions here replay a command block from
//! `tests/ps3autotests/tests/rsx/methods_pfifo_puller/methods_pfifo_puller.cpp`
//! and check CellGov's answer against the line the PS3 printed in the
//! `.expected` file beside it. A test whose words are not the
//! fixture's says so in its own comment, and names what the fixture
//! does and does not show.
//!
//! The fixture drives the pusher through raw lpar windows and reads
//! the results back out of them. It therefore settles:
//!
//! - the header encoding,
//! - the four NV406E method ids,
//! - the byte-for-byte shape of a semaphore release.
//!
//! It settles nothing about `region::DRIVER_INFO_OFFSET`,
//! `region::REPORTS_OFFSET` or `region::CONTEXT_RESERVATION`. Those
//! constants are offsets from a context base CellGov's `sys_rsx` model
//! picks. The fixture's windows are absolute lpar addresses its author
//! hardcoded, under a standing TODO to read them back instead. The
//! fixture also never touches the driver-info window it declares --
//! only the DMA-control and reports windows produce output -- so even
//! its own placement of that window is untested.

use super::*;
use crate::rsx::method::{
    register_nv406e_label_handlers, register_nv406e_reference_handler, NV406E_SEMAPHORE_ACQUIRE,
    NV406E_SEMAPHORE_OFFSET, NV406E_SEMAPHORE_RELEASE, NV406E_SET_REFERENCE, NV_COUNT_SHIFT,
};
use cellgov_mem::GuestMemory;

const FIFO_BASE: u32 = 0x1000;

/// The fixture's own `METHOD(offset, count)` macro.
fn method(offset: u16, count: u16) -> u32 {
    ((count as u32) << 18) | (offset as u32)
}

fn drain(words: &[u32]) -> (RsxFifoCursor, Vec<Effect>, RsxAdvanceOutcome) {
    let mut memory = GuestMemory::new(0x4000);
    let len = (words.len() as u64) * 4;
    let range = ByteRange::new(GuestAddr::new(FIFO_BASE as u64), len).unwrap();
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for &w in words {
        bytes.extend_from_slice(&w.to_be_bytes());
    }
    memory.apply_commit(range, &bytes).unwrap();

    let mut cursor = RsxFifoCursor::new();
    cursor.set_get(FIFO_BASE);
    cursor.set_put(FIFO_BASE + (words.len() as u32) * 4);

    let mut table = NvMethodTable::new();
    register_nv406e_label_handlers(&mut table).unwrap();
    register_nv406e_reference_handler(&mut table).unwrap();

    let mut emitted: Vec<Effect> = Vec::new();
    let mut sem_offset = 0u32;
    let mut call_stack = RsxCallStack::new();
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
    (cursor, emitted, outcome)
}

fn label_writes(effects: &[Effect]) -> Vec<(u32, u32)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::RsxLabelWrite { offset, value } => Some((*offset, *value)),
            _ => None,
        })
        .collect()
}

#[test]
fn fifo_header_packs_the_argument_count_at_bit_18() {
    // The fixture builds every header with `(count << 18) | offset`
    // and a PS3 executes the result, so the shift is hardware-fixed.
    assert_eq!(NV_COUNT_SHIFT, 18);
    assert_eq!(method(0x50, 1), (1u32 << NV_COUNT_SHIFT) | 0x50);
}

#[test]
fn set_reference_lands_the_word_the_console_reads_back() {
    // Fixture: one SET_REFERENCE of 0x12345678, then the PS3 prints
    // "Reference: 0x12345678" read from the control block.
    let (cursor, emitted, outcome) = drain(&[method(0x50, 1), 0x1234_5678]);
    assert!(outcome.reached_put());
    assert_eq!(cursor.current_reference(), 0x1234_5678);
    assert!(
        label_writes(&emitted).is_empty(),
        "a reference write is not a label write"
    );
}

#[test]
fn the_four_channel_method_ids_are_the_ones_the_console_accepted() {
    // The words the fixture reads back attest 0x50, 0x64 and 0x6C.
    // The ordering attests 0x68: the CPU stores 0 into the second
    // semaphore after the fixture submits the release, and the console
    // still prints 1. The puller therefore parked on the acquire and
    // ran the release afterwards.
    assert_eq!(NV406E_SET_REFERENCE, 0x0050);
    assert_eq!(NV406E_SEMAPHORE_OFFSET, 0x0064);
    assert_eq!(NV406E_SEMAPHORE_ACQUIRE, 0x0068);
    assert_eq!(NV406E_SEMAPHORE_RELEASE, 0x006C);
}

#[test]
fn a_semaphore_release_writes_its_argument_verbatim_at_the_named_offset() {
    // Fixture, in order: it zeroes both semaphores at offsets 0x10 and
    // 0x20, then sets 0x10 to 1 and 0x20 to 0xFFFFFFFF. The PS3 reads
    // back "0x00000001" and "0xFFFFFFFF", so the value crosses
    // unchanged -- no swap, no scaling of the offset.
    //
    // The fixture's first block opens with method 0x60
    // (NV406E_SET_CONTEXT_DMA_SEMAPHORE, argument 0x66616661). CellGov
    // registers no handler for it, and the console output shows
    // nothing that depends on it, so this replay omits it.
    let (_, emitted, outcome) = drain(&[
        method(0x64, 1),
        0x10,
        method(0x6C, 1),
        0x0,
        method(0x64, 1),
        0x20,
        method(0x6C, 1),
        0x0,
        method(0x64, 1),
        0x10,
        method(0x6C, 1),
        0x1,
        method(0x64, 1),
        0x20,
        method(0x6C, 1),
        0xFFFF_FFFF,
    ]);
    assert!(outcome.reached_put());
    assert_eq!(
        label_writes(&emitted),
        vec![(0x10, 0x0), (0x20, 0x0), (0x10, 0x1), (0x20, 0xFFFF_FFFF),]
    );
}

#[test]
fn an_offset_set_once_serves_every_release_that_follows_it() {
    // This test writes its own words and values; it replays no fixture
    // block. The fixture writes an offset before every release it
    // issues, so the console output says nothing about whether the
    // offset register persists past one release. A sticky register is
    // CellGov's reading of the method and is unanchored.
    let (_, emitted, _) = drain(&[
        method(0x64, 1),
        0x30,
        method(0x6C, 1),
        0xAA,
        method(0x6C, 1),
        0xBB,
    ]);
    assert_eq!(label_writes(&emitted), vec![(0x30, 0xAA), (0x30, 0xBB)]);
}

#[test]
fn an_acquire_is_counted_unknown_and_does_not_hold_back_the_release_after_it() {
    // Fixture block 4: set offset 0x10, acquire 2, set offset 0x20,
    // release 1. On the console the puller parked on the acquire until
    // the CPU stored 2 at 0x10, and only then did 0x20 read back
    // "0x00000001". CellGov registers no handler for the acquire, so
    // the drain counts one unknown method and the release crosses at
    // once: same final word, no wait. The unknown count is the witness
    // that CellGov drops the gate.
    let (_, emitted, outcome) = drain(&[
        method(0x64, 1),
        0x10,
        method(0x68, 1),
        0x2,
        method(0x64, 1),
        0x20,
        method(0x6C, 1),
        0x1,
    ]);
    assert!(outcome.reached_put());
    assert_eq!(outcome.methods_dispatched, 3);
    assert_eq!(outcome.methods_unknown, 1);
    assert_eq!(label_writes(&emitted), vec![(0x20, 0x1)]);
}
