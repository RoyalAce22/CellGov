//! The `lswi` / `lswx` / `stswi` / `stswx` string cores.

use crate::exec::memory_helpers::{buffer_store, check_store, load_ze, LoadPort, Width};
use crate::exec::ExecuteVerdict;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// Whether GPR `r` is one of the registers an `n`-byte load string fills
/// from `rt`: four bytes to a register, wrapping at r31 -> r0.
pub(crate) fn string_loads_into(rt: u8, n: usize, r: u8) -> bool {
    (usize::from(r) + 32 - usize::from(rt)) % 32 < n.div_ceil(4)
}

/// The bytes an invalid-form `n`-byte load string from `rt` loads before
/// it reaches the first register of `collide` that it fills: four for
/// each register ahead of it.
///
/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
pub(crate) fn bytes_before_collision(rt: u8, n: usize, collide: &[u8]) -> u64 {
    let ahead = collide
        .iter()
        .filter(|&&r| string_loads_into(rt, n, r))
        .map(|&r| 4 * ((usize::from(r) + 32 - usize::from(rt)) % 32))
        .min()
        .unwrap_or(0);
    ahead.min(n) as u64
}

/// `lswi` / `lswx` core: read `n` bytes from `base` and pack
/// MSB-first four-per-register into successive GPRs starting at
/// `rt_start`, wrapping at r31 -> r0. Zero-length is a no-op. The core
/// loads every byte before it writes a register, so a fault writes none.
pub(super) fn string_load(
    state: &mut PpuState,
    port: &mut LoadPort<'_, '_>,
    rt_start: usize,
    base: u64,
    n: usize,
) -> ExecuteVerdict {
    let mut bytes = [0u8; 128];
    let Some(bytes) = bytes.get_mut(..n) else {
        return ExecuteVerdict::Fault(crate::exec::PpuFault::Internal("string length"));
    };
    for (i, byte) in bytes.iter_mut().enumerate() {
        match load_ze(port, base.wrapping_add(i as u64), Width::B1) {
            Ok(v) => *byte = v as u8,
            Err(e) => return ExecuteVerdict::MemFault(e),
        }
    }
    for (i, word) in bytes.chunks(4).enumerate() {
        let value = word
            .iter()
            .enumerate()
            .fold(0u64, |acc, (j, &b)| acc | (u64::from(b) << ((3 - j) * 8)));
        state.set_gpr((rt_start + i) % 32, value);
    }
    ExecuteVerdict::Continue
}

/// `stswi` / `stswx` core: store `n` bytes from `base`, extracting
/// MSB-first four-per-register from successive GPRs starting at
/// `rs_start` and wrapping at r31 -> r0. The address and capacity
/// checks run before the first byte goes to the buffer, so neither a
/// fault nor a `BufferFull` leaves part of the string staged.
pub(super) fn string_store(
    state: &mut PpuState,
    region_views: &[cellgov_mem::RegionView<'_>],
    store_buf: &mut StoreBuffer,
    rs_start: usize,
    base: u64,
    n: usize,
) -> ExecuteVerdict {
    if n == 0 {
        return ExecuteVerdict::Continue;
    }
    for i in 0..n {
        if let Err(e) = check_store(region_views, base.wrapping_add(i as u64), 1) {
            return ExecuteVerdict::StoreFault(e);
        }
    }
    if !store_buf.has_capacity_for(n) {
        return ExecuteVerdict::BufferFull;
    }
    let mut reg = rs_start % 32;
    let mut byte_idx = 0usize;
    for i in 0..n {
        let shift = (3 - byte_idx) * 8;
        let byte = ((state.gpr[reg] >> shift) & 0xFF) as u8;
        let v = buffer_store(
            store_buf,
            region_views,
            base.wrapping_add(i as u64),
            1,
            byte as u64,
        );
        debug_assert!(
            v != ExecuteVerdict::BufferFull,
            "string-store byte failed after capacity pre-check"
        );
        if v != ExecuteVerdict::Continue {
            return v;
        }
        byte_idx += 1;
        if byte_idx == 4 {
            byte_idx = 0;
            reg = (reg + 1) % 32;
        }
    }
    ExecuteVerdict::Continue
}
