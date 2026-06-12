//! Shared test helpers for `Lv2Host` dispatch tests.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::GuestTicks;

use crate::host::{Lv2Host, Lv2Runtime};
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;

pub(super) struct FakeRuntime {
    memory: GuestMemory,
    tick: GuestTicks,
    /// `Some(b)` forces `writable()` to return `b` regardless of
    /// the addr/len bounds check.
    writable_override: Option<bool>,
    /// Per-address `writable()` override; takes precedence over
    /// `writable_override`.
    writable_at: std::collections::BTreeMap<u64, bool>,
}

impl FakeRuntime {
    pub(super) fn new(size: usize) -> Self {
        Self {
            memory: GuestMemory::new(size),
            tick: GuestTicks::ZERO,
            writable_override: None,
            writable_at: std::collections::BTreeMap::new(),
        }
    }

    pub(super) fn with_memory(memory: GuestMemory) -> Self {
        Self {
            memory,
            tick: GuestTicks::ZERO,
            writable_override: None,
            writable_at: std::collections::BTreeMap::new(),
        }
    }

    pub(super) fn with_tick(mut self, tick: GuestTicks) -> Self {
        self.tick = tick;
        self
    }

    /// Override `writable()` to return `value` for every query.
    pub(super) fn with_writable_override(mut self, value: bool) -> Self {
        self.writable_override = Some(value);
        self
    }

    /// Register a per-address writability override. Queries to `addr`
    /// return `value`; other addresses fall through to
    /// `writable_override` or the default bounds check.
    pub(super) fn with_writable_at(mut self, addr: u64, value: bool) -> Self {
        self.writable_at.insert(addr, value);
        self
    }
}

impl Lv2Runtime for FakeRuntime {
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]> {
        let start = addr as usize;
        let end = start.checked_add(len)?;
        let bytes = self.memory.as_bytes();
        if end <= bytes.len() {
            Some(&bytes[start..end])
        } else {
            None
        }
    }

    fn current_tick(&self) -> GuestTicks {
        self.tick
    }

    fn read_committed_until(&self, addr: u64, max_len: usize, terminator: u8) -> Option<&[u8]> {
        let bytes = self.memory.as_bytes();
        let start = addr as usize;
        let end = start.checked_add(max_len)?.min(bytes.len());
        if start >= bytes.len() {
            return None;
        }
        let window = &bytes[start..end];
        let nul_pos = window.iter().position(|&b| b == terminator)?;
        Some(&window[..nul_pos])
    }

    fn writable(&self, addr: u64, len: usize) -> bool {
        if let Some(&per_addr) = self.writable_at.get(&addr) {
            return per_addr;
        }
        if let Some(forced) = self.writable_override {
            return forced;
        }
        let Some(end) = (addr).checked_add(len as u64) else {
            return false;
        };
        let bytes = self.memory.as_bytes();
        end <= bytes.len() as u64
    }
}

/// Extract the big-endian u32 payload from a `SharedWriteIntent`.
pub(super) fn extract_write_u32(effect: &Effect) -> u32 {
    match effect {
        Effect::SharedWriteIntent { bytes, .. } => {
            let b = bytes.bytes();
            assert_eq!(b.len(), 4);
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

/// Seed a primary PPU thread mapped to `unit_id`.
pub(super) fn seed_primary_ppu(host: &mut Lv2Host, unit_id: UnitId) {
    host.seed_primary_ppu_thread(
        unit_id,
        PpuThreadAttrs {
            entry: 0,
            arg: 0,
            stack_base: 0,
            stack_size: 0,
            priority: 0,
            tls_base: 0,
        },
    );
}

/// Address where [`fake_runtime_with_valid_sync_attr`] seeds a
/// fully-initialized 24-byte `sys_*_attribute_t` header so creation
/// dispatches accept the pointer.
pub(super) const VALID_SYNC_ATTR_PTR: u32 = 0x800;

/// Build a `FakeRuntime` whose guest memory has a valid 24-byte
/// `sys_*_attribute_t` header (protocol = SYS_SYNC_FIFO at +0, type =
/// SYS_SYNC_WAITER_SINGLE at +20) at [`VALID_SYNC_ATTR_PTR`]. Use when
/// the test exercises a happy-path create that needs the LV2 host to
/// validate the attribute fields rather than reject NULL/zero attrs.
pub(super) fn fake_runtime_with_valid_sync_attr(size: usize) -> FakeRuntime {
    let mut mem = GuestMemory::new(size);
    let mut attr = [0u8; 24];
    attr[0..4].copy_from_slice(&0x1u32.to_be_bytes()); // protocol = SYS_SYNC_FIFO
    attr[20..24].copy_from_slice(&0x10000u32.to_be_bytes()); // type = SYS_SYNC_WAITER_SINGLE
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(VALID_SYNC_ATTR_PTR as u64), 24).unwrap(),
        &attr,
    )
    .unwrap();
    FakeRuntime::with_memory(mem)
}

pub(super) fn primary_attrs() -> PpuThreadAttrs {
    PpuThreadAttrs {
        entry: 0x10_0000,
        arg: 0,
        stack_base: 0xD000_0000,
        stack_size: 0x10000,
        priority: 1000,
        tls_base: 0x0020_0000,
    }
}

/// Lay out the two-step indirection that `_sys_ppu_thread_create`
/// expects in guest memory: a `ppu_thread_param_t` at `param_addr`
/// whose first u32 points at an OPD planted 8 bytes after it, and
/// that OPD's `{ code, toc }` pair as 4-byte BE u32s each.
///
/// # Panics
///
/// Panics if `param_addr + 8` overflows `u32` or if `param_addr +
/// 16` exceeds the 0x1_0000-byte arena.
pub(super) fn opd_runtime(param_addr: u32, entry_code: u32, entry_toc: u32) -> FakeRuntime {
    let mut mem = GuestMemory::new(0x1_0000);
    let opd_addr = param_addr
        .checked_add(8)
        .expect("opd_runtime: param_addr + 8 must fit in u32");
    assert!(
        opd_addr.checked_add(8).is_some_and(|end| end <= 0x1_0000),
        "opd_runtime: OPD region [{opd_addr:#x}, {opd_addr:#x}+8) must stay within the \
         0x1_0000-byte arena; pick a smaller param_addr"
    );

    let mut param_bytes = [0u8; 8];
    param_bytes[0..4].copy_from_slice(&opd_addr.to_be_bytes());
    let param_range = ByteRange::new(GuestAddr::new(param_addr as u64), 8).unwrap();
    mem.apply_commit(param_range, &param_bytes).unwrap();

    let mut opd_bytes = [0u8; 8];
    opd_bytes[0..4].copy_from_slice(&entry_code.to_be_bytes());
    opd_bytes[4..8].copy_from_slice(&entry_toc.to_be_bytes());
    let opd_range = ByteRange::new(GuestAddr::new(opd_addr as u64), 8).unwrap();
    mem.apply_commit(opd_range, &opd_bytes).unwrap();

    FakeRuntime::with_memory(mem)
}

pub(super) fn create_mutex_host(host: &mut Lv2Host, src: UnitId, rt: &FakeRuntime) -> u32 {
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        src,
        rt,
    );
    match &created {
        crate::dispatch::Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[cfg(test)]
#[path = "test_support_tests.rs"]
mod tests;
