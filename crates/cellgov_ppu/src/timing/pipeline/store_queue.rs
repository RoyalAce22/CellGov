//! The store queue: the 16-byte entries a store takes, the room it waits
//! for, and the bytes each entry keeps until it writes them into the
//! cache.

use super::{Op, Operand, Queued, Tables, PENDING_ENTRIES};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The 16-byte block that holds `ea`.
fn block_of(ea: u64) -> u64 {
    ea / k::PPU_STORE_ENTRY_BYTES
}

/// The bytes of block `block` in `[ea, end)`, bit `i` for byte `i`.
pub(super) fn bytes_in(block: u64, ea: u64, end: u64) -> u16 {
    let base = block.wrapping_mul(k::PPU_STORE_ENTRY_BYTES);
    let lo = ea.max(base) - base;
    let hi = end
        .min(base.saturating_add(k::PPU_STORE_ENTRY_BYTES))
        .saturating_sub(base);
    if lo >= hi {
        return 0;
    }
    ((1u32 << hi) - (1u32 << lo)) as u16
}

/// A store's parts as (address, bytes), one per 16-byte block it writes.
///
/// [PPC-Book1 p:49 s:3.3.6] an `stswx` with a byte count of zero stores
/// no bytes, so it has no part.
fn store_parts(ea: u64, len: u8) -> impl Iterator<Item = (u64, u8)> {
    let bytes = u64::from(len);
    let mut at = 0;
    std::iter::from_fn(move || {
        if at >= bytes {
            return None;
        }
        let here = ea.wrapping_add(at);
        let room = k::PPU_STORE_ENTRY_BYTES - here % k::PPU_STORE_ENTRY_BYTES;
        let part = (bytes - at).min(room);
        at += part;
        Some((here, part as u8))
    })
}

/// Whether a store's first part at `ea` gathers into the youngest entry.
///
/// It does when that entry holds bytes of the same 16-byte block. In the
/// `ppu_issue_store_queue_drain` capture, 32 `std` two to a block take 90
/// cycles an iteration, and 32 `std` one to a block take 253.
fn gathers(tables: &Tables, ea: u64) -> bool {
    let youngest = &tables.stores[(tables.next_store + PENDING_ENTRIES - 1) % PENDING_ENTRIES];
    youngest.mask != 0 && youngest.block == block_of(ea)
}

/// The new entries a store takes: one per 16-byte block, less one that gathers.
fn new_entries(tables: &Tables, ea: u64, len: u8) -> usize {
    let parts = store_parts(ea, len).count();
    parts.saturating_sub(usize::from(parts != 0 && gathers(tables, ea)))
}

/// The earliest cycle from `t` at which a store has room for its new entries.
///
/// The store waits:
/// - while the store queue is full, until the oldest entries leave it;
/// - while the pending entries are full, until the oldest entry writes
///   its bytes into the cache.
///
/// No entry is lost. In the `ppu_issue_store_queue_full` capture, 16
/// `std` and a load of the first one's bytes take 49 cycles an iteration.
/// With 17 `std` they take 52, so the load still waits for its store.
///
/// [CBE-Handbook p:762 s:A.7.4] a load that hits the store queue waits
/// until the store has written its data into the cache.
pub(super) fn queue_room(tables: &Tables, t: u64, ea: u64, len: u8) -> u64 {
    let new = new_entries(tables, ea, len);
    let slot = (0..new.min(k::PPU_STORE_QUEUE_ENTRIES))
        .map(|i| tables.slots[(tables.next_slot + i) % k::PPU_STORE_QUEUE_ENTRIES])
        .max()
        .unwrap_or(0);
    let written = (0..new.min(PENDING_ENTRIES))
        .map(|i| &tables.stores[(tables.next_store + i) % PENDING_ENTRIES])
        .filter(|s| s.mask != 0)
        .map(|s| s.done)
        .max()
        .unwrap_or(0);
    t.max(slot).max(written)
}

/// Puts a store of the bytes of `operand`, issued at `t`, into the store
/// queue.
///
/// The caches store the line of each byte at that byte's real address,
/// which the tail gives for the bytes on the next page. The store queue's
/// entries take their blocks from the real address of the first byte
/// alone.
///
/// The store takes one entry per 16-byte block. Its first part gathers
/// into the youngest entry when that entry holds the same block.
///
/// An entry keeps the bytes its stores wrote, so a load waits only for
/// the bytes it reads. When a store gathers into an entry that wrote its
/// bytes into the cache, the entry drops those bytes. In the
/// `ppu_issue_store_gather_hit` capture, an `stw`, 96 `addi`, an `stw` to
/// the same block and a load of the first word take 100 cycles an
/// iteration. With the second `stw` to another line they take 103.
///
/// Each entry takes a slot of the store queue until it leaves, and
/// pends until it writes its bytes into the cache. The entries of one
/// store leave [`PPU_STORE_SLOT_DRAIN_CYCLES`] apart and write
/// [`PPU_STORE_DRAIN_CYCLES`] apart.
///
/// [`PPU_STORE_SLOT_DRAIN_CYCLES`]: k::PPU_STORE_SLOT_DRAIN_CYCLES
/// [`PPU_STORE_DRAIN_CYCLES`]: k::PPU_STORE_DRAIN_CYCLES
#[inline]
pub(super) fn queue_store(tables: &mut Tables, t: u64, op: &Op, operand: Operand) {
    tables.caches.store(operand);
    let Operand { ra, len, .. } = operand;
    // [CBE-Handbook p:134 s:6.1.2.2] the CIU store queue holds
    // 16-byte entries. [CBE-Handbook p:144 s:6.1.5.7] the L2
    // store queue that takes each CIU request unloads first in,
    // first out. The interval between two entries of one store
    // is fitted to the `ppu_issue_multiple` capture. Each store
    // starts its own chain: in the `ppu_issue_load_hit_store`
    // capture, a store to the loaded line after three stores to
    // other lines takes as long as that store alone.
    let first = t + u64::from(op.span.saturating_sub(1)) + k::PPU_LOAD_HIT_STORE_CYCLES;
    let (mut done, mut leaves) = (first, first);
    let gather = gathers(tables, ra);
    let slots = k::PPU_STORE_QUEUE_ENTRIES;
    for (index, (at, part)) in store_parts(ra, len).enumerate() {
        let block = block_of(at);
        let mask = bytes_in(block, at, at.saturating_add(u64::from(part)));
        if index == 0 && gather {
            let youngest =
                &mut tables.stores[(tables.next_store + PENDING_ENTRIES - 1) % PENDING_ENTRIES];
            if youngest.done <= t {
                youngest.mask = 0;
            }
            youngest.mask |= mask;
            youngest.done = youngest.done.max(done);
            let slot = &mut tables.slots[(tables.next_slot + slots - 1) % slots];
            *slot = (*slot).max(leaves);
        } else {
            let entry = &mut tables.stores[tables.next_store];
            debug_assert!(
                entry.mask == 0 || entry.done <= t,
                "a store overwrites an entry that has not written its bytes"
            );
            *entry = Queued { block, mask, done };
            tables.next_store = (tables.next_store + 1) % PENDING_ENTRIES;
            debug_assert!(
                tables.slots[tables.next_slot] <= t,
                "a store takes a slot of the store queue that is not free"
            );
            tables.slots[tables.next_slot] = leaves;
            tables.next_slot = (tables.next_slot + 1) % slots;
        }
        done += k::PPU_STORE_DRAIN_CYCLES;
        leaves += k::PPU_STORE_SLOT_DRAIN_CYCLES;
    }
}
