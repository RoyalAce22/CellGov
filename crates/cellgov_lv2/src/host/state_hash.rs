//! FNV-1a state-hash contribution for [`Lv2Host`].
//!
//! # Cross-module contract
//!
//! Types that contribute to the host's `state_hash` must be folded
//! through FNV-1a via their `.raw()` (or `.to_le_bytes()`) accessor,
//! not via `std::hash::Hash`. The runtime's `sync_state_hash` is
//! cross-build stable, so any hasher whose output depends on
//! compiler version / build configuration (`DefaultHasher`,
//! `RandomState`) is forbidden. New fields land their contribution
//! in [`Lv2Host::state_hash`]; gating on `!is_empty()` keeps the
//! hash stable at boot until a primitive is actually used.

use crate::ppu_thread::ThreadStackAllocator;

use super::Lv2Host;

impl Lv2Host {
    /// FNV-1a of all committed LV2 host state; folded into the
    /// runtime's `sync_state_hash` at every commit boundary.
    ///
    /// # Gating
    /// Per-primitive tables and the child-stack allocator contribute
    /// only when non-empty / past their sentinel. `next_kernel_id`
    /// and `mem_alloc_ptr` always contribute, so a
    /// created-then-destroyed primitive still advances the hash via
    /// allocator state once the table empties again.
    ///
    /// # Cost
    /// Linear in the number of live primitives plus per-thread
    /// lwmutex-hold and callback-parent map sizes; runs once per
    /// commit boundary.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [self.content.state_hash(), self.groups.state_hash()] {
            hasher.write(&source.to_le_bytes());
        }
        hasher.write(&self.next_kernel_id.to_le_bytes());
        hasher.write(&self.mem_alloc_ptr.to_le_bytes());
        hasher.write(&self.rsx_mem_alloc_ptr.to_le_bytes());
        hasher.write(&self.rsx_mem_handle_counter.to_le_bytes());
        hasher.write(&self.rsx_context.state_hash().to_le_bytes());
        if !self.ppu_threads.is_empty() {
            hasher.write(&self.ppu_threads.state_hash().to_le_bytes());
        }
        if !self.tls_template.is_empty() {
            hasher.write(&self.tls_template.state_hash().to_le_bytes());
        }
        if let Some(peek) = self.stack_allocator.peek_next(0x10) {
            if peek != ThreadStackAllocator::CHILD_STACK_BASE {
                hasher.write(&peek.to_le_bytes());
            }
        }
        if !self.lwmutexes.is_empty() {
            hasher.write(&self.lwmutexes.state_hash().to_le_bytes());
        }
        if !self.mutexes.is_empty() {
            hasher.write(&self.mutexes.state_hash().to_le_bytes());
        }
        if !self.semaphores.is_empty() {
            hasher.write(&self.semaphores.state_hash().to_le_bytes());
        }
        if !self.event_queues.is_empty() {
            hasher.write(&self.event_queues.state_hash().to_le_bytes());
        }
        if !self.event_flags.is_empty() {
            hasher.write(&self.event_flags.state_hash().to_le_bytes());
        }
        if !self.conds.is_empty() {
            hasher.write(&self.conds.state_hash().to_le_bytes());
        }
        if !self.lwmutex_holds.is_empty() {
            hasher.write(&(self.lwmutex_holds.len() as u64).to_le_bytes());
            for (tid, count) in &self.lwmutex_holds {
                hasher.write(&tid.raw().to_le_bytes());
                hasher.write(&count.to_le_bytes());
            }
        }
        if !self.fs_store.is_empty() {
            hasher.write(&self.fs_store.state_hash().to_le_bytes());
        }
        if let Some(fw) = self.firmware_identity() {
            hasher.write(&fw.image_version_hash.to_le_bytes());
            hasher.write(&fw.pup_sha256_bytes);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::primary_attrs;
    use crate::ppu_thread::TlsTemplate;
    use cellgov_event::UnitId;

    #[test]
    fn state_hash_unchanged_when_ppu_table_empty() {
        let fresh = Lv2Host::new();
        assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
    }

    #[test]
    fn state_hash_changes_after_primary_seed() {
        let pre_seed = Lv2Host::new().state_hash();
        let mut seeded = Lv2Host::new();
        seeded.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        assert_ne!(pre_seed, seeded.state_hash());
    }

    #[test]
    fn state_hash_unchanged_when_tls_template_empty() {
        let fresh = Lv2Host::new();
        assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
    }

    #[test]
    fn state_hash_changes_when_holds_inserted_then_returns_to_baseline() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let baseline = host.state_hash();
        let tid = host.ppu_thread_id_for_unit(UnitId::new(0)).unwrap();
        host.lwmutex_holds_inc(tid);
        assert_ne!(baseline, host.state_hash());
        host.lwmutex_holds_dec(tid);
        assert_eq!(baseline, host.state_hash());
    }

    #[test]
    fn state_hash_changes_after_tls_template_set() {
        let pre = Lv2Host::new().state_hash();
        let mut host = Lv2Host::new();
        host.set_tls_template(TlsTemplate::new(vec![0x11, 0x22], 0x80, 0x10, 0x1000));
        assert_ne!(pre, host.state_hash());
    }

    #[test]
    fn state_hash_unchanged_when_no_child_stack_allocated() {
        let fresh = Lv2Host::new();
        assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
    }

    #[test]
    fn state_hash_changes_after_child_stack_allocated() {
        let pre = Lv2Host::new().state_hash();
        let mut host = Lv2Host::new();
        let _ = host.allocate_child_stack(0x10_000, 0x10).unwrap();
        assert_ne!(pre, host.state_hash());
    }

    #[test]
    fn state_hash_changes_after_firmware_identity_set() {
        let pre = Lv2Host::new().state_hash();
        let mut host = Lv2Host::new();
        host.set_firmware_identity("4.85", [0u8; 32]);
        assert_ne!(pre, host.state_hash());
    }

    #[test]
    fn state_hash_differs_between_two_firmware_versions() {
        let mut a = Lv2Host::new();
        let mut b = Lv2Host::new();
        a.set_firmware_identity("4.85", [0u8; 32]);
        b.set_firmware_identity("4.86", [0u8; 32]);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_equal_across_two_runs_of_same_firmware() {
        let mut a = Lv2Host::new();
        let mut b = Lv2Host::new();
        let digest: [u8; 32] = [0x42; 32];
        a.set_firmware_identity("4.85", digest);
        b.set_firmware_identity("4.85", digest);
        assert_eq!(a.state_hash(), b.state_hash());
    }
}
