//! The `lswi` / `lswx` / `stswi` / `stswx` string cores.

use crate::exec::memory_helpers::{buffer_store, load_ze, LoadPort, Width};
use crate::exec::ExecuteVerdict;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

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
        return ExecuteVerdict::Fault(crate::exec::PpuFault::InvalidForm("string length"));
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
/// `rs_start` and wrapping at r31 -> r0. Capacity pre-check
/// prevents partial commit on `BufferFull`.
pub(super) fn string_store(
    state: &mut PpuState,
    store_buf: &mut StoreBuffer,
    rs_start: usize,
    base: u64,
    n: usize,
) -> ExecuteVerdict {
    if n == 0 {
        return ExecuteVerdict::Continue;
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
            state,
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
