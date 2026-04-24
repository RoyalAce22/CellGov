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
}

impl FakeRuntime {
    pub(super) fn new(size: usize) -> Self {
        Self {
            memory: GuestMemory::new(size),
            tick: GuestTicks::ZERO,
        }
    }

    pub(super) fn with_memory(memory: GuestMemory) -> Self {
        Self {
            memory,
            tick: GuestTicks::ZERO,
        }
    }

    pub(super) fn with_tick(mut self, tick: GuestTicks) -> Self {
        self.tick = tick;
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

pub(super) fn opd_runtime(opd_addr: u32, entry_code: u64, entry_toc: u64) -> FakeRuntime {
    let mut mem = GuestMemory::new(0x1_0000);
    let range = ByteRange::new(GuestAddr::new(opd_addr as u64), 16).unwrap();
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&entry_code.to_be_bytes());
    bytes[8..16].copy_from_slice(&entry_toc.to_be_bytes());
    mem.apply_commit(range, &bytes).unwrap();
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
mod tests {
    use super::*;

    #[test]
    fn fake_runtime_reads_committed_memory() {
        let rt = FakeRuntime::new(256);
        assert!(rt.read_committed(0, 4).is_some());
        assert!(rt.read_committed(252, 4).is_some());
        assert!(rt.read_committed(253, 4).is_none());
        assert!(rt.read_committed(0, 0).is_some());
    }
}
