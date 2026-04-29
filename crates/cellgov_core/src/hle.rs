//! HLE router chaining per-library dispatch modules.
//!
//! Filenames match the canonical Sony library names so a visual grep
//! against RPCS3's source tree lines up one-to-one; Rust module
//! identifiers stay snake_case via `#[path]`.
//!
//! ## Unclaimed-NID policy
//!
//! When no module claims a NID, the router sets r3 = CELL_OK (0) with
//! no side effects and records the NID in
//! [`HleState::unclaimed_nids`] (one-shot stderr on first occurrence).
//! Downstream observability can walk the counter to attribute
//! corrupted state to a specific unimplemented library entry.

use cellgov_event::UnitId;

use crate::hle::cell_gcm_sys::GcmState;
use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

#[path = "hle/cellGcmSys.rs"]
pub(crate) mod cell_gcm_sys;
#[path = "hle/cellSpurs.rs"]
pub(crate) mod cell_spurs;
#[path = "hle/cellSysutil.rs"]
pub(crate) mod cell_sysutil;
pub mod context;
#[path = "hle/sysPrxForUser.rs"]
pub(crate) mod sys_prx_for_user;

/// HLE-specific bookkeeping bundled off the `Runtime` struct.
pub(crate) struct HleState {
    pub nids: std::collections::BTreeMap<u32, u32>,
    /// Bump-allocator base (most recent `Runtime::set_hle_heap_base`).
    /// The watermark threshold check subtracts this from `heap_ptr`
    /// to report bytes handed out rather than the raw cursor.
    pub heap_base: u32,
    pub heap_ptr: u32,
    /// Peak `heap_ptr`. `heap_alloc` emits a one-shot stderr warning
    /// when `heap_watermark - heap_base` crosses 1 MiB / 10 MiB /
    /// 100 MiB.
    pub heap_watermark: u32,
    /// Bits: 0x1 = 1 MiB, 0x2 = 10 MiB, 0x4 = 100 MiB.
    pub heap_warning_mask: u8,
    /// Monotonic kernel-object ID counter. Starts above zero so a
    /// zero-initialized guest field is distinguishable from an
    /// allocated ID.
    pub next_id: u32,
    pub gcm: GcmState,
    /// NIDs that no module claimed, keyed by NID with a call count.
    /// First occurrence of each NID emits a stderr line in
    /// `dispatch_hle`.
    pub unclaimed_nids: std::collections::BTreeMap<u32, usize>,
    /// NIDs that were claimed and routed to a handler but produced
    /// no observable mutation before the adapter dropped. Populated
    /// from [`context::RuntimeHleAdapter`]'s `Drop` in both debug
    /// and release. A non-zero entry here is a handler whose
    /// register state leaked straight through to the guest without
    /// a fresh r3.
    pub handlers_without_mutation: std::collections::BTreeMap<u32, usize>,
}

impl HleState {
    pub(crate) fn new() -> Self {
        Self {
            nids: std::collections::BTreeMap::new(),
            heap_base: 0,
            heap_ptr: 0,
            heap_watermark: 0,
            heap_warning_mask: 0,
            next_id: 0x8000_0001,
            gcm: GcmState::default(),
            unclaimed_nids: std::collections::BTreeMap::new(),
            handlers_without_mutation: std::collections::BTreeMap::new(),
        }
    }
}

impl Runtime {
    /// Dispatch an HLE import call by NID.
    ///
    /// See the module-level "Unclaimed-NID policy" for the fallback
    /// contract.
    pub(crate) fn dispatch_hle(&mut self, source: UnitId, nid: u32, args: &[u64; 9]) {
        let handled = sys_prx_for_user::dispatch(self, source, nid, args)
            .or_else(|| cell_gcm_sys::dispatch(self, source, nid, args))
            .or_else(|| cell_sysutil::dispatch(self, source, nid, args))
            .or_else(|| cell_spurs::dispatch(self, source, nid, args));
        if handled.is_none() {
            let entry = self.hle.unclaimed_nids.entry(nid).or_insert(0);
            if *entry == 0 {
                eprintln!(
                    "HLE dispatch: unclaimed NID {nid:#010x} called from {source:?}; \
                     returning CELL_OK with no side effects (silent divergence risk)"
                );
            }
            *entry += 1;
            RuntimeHleAdapter {
                memory: &mut self.memory,
                registry: &mut self.registry,
                heap_base: self.hle.heap_base,
                heap_ptr: &mut self.hle.heap_ptr,
                heap_watermark: &mut self.hle.heap_watermark,
                heap_warning_mask: &mut self.hle.heap_warning_mask,
                next_id: &mut self.hle.next_id,
                source,
                nid,
                mutated: false,
                handlers_without_mutation: &mut self.hle.handlers_without_mutation,
            }
            .set_return(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::hle::cell_gcm_sys::tiled_pitch_lookup;
    use cellgov_ps3_abi::nid::cell_gcm_sys as gcm_nid;
    use cellgov_ps3_abi::nid::sys_prx_for_user as sys_nid;

    #[test]
    fn tiled_pitch_exact_boundary() {
        assert_eq!(tiled_pitch_lookup(0x200), 0x200);
        assert_eq!(tiled_pitch_lookup(0x300), 0x300);
        assert_eq!(tiled_pitch_lookup(0x10000), 0x10000);
    }

    #[test]
    fn tiled_pitch_between_entries() {
        assert_eq!(tiled_pitch_lookup(0x250), 0x300);
        assert_eq!(tiled_pitch_lookup(0x1), 0x200);
        assert_eq!(tiled_pitch_lookup(0x801), 0xA00);
    }

    #[test]
    fn tiled_pitch_just_past_first_window_returns_next() {
        assert_eq!(tiled_pitch_lookup(0x201), 0x300);
    }

    #[test]
    fn tiled_pitch_zero_returns_zero() {
        assert_eq!(tiled_pitch_lookup(0), 0);
    }

    #[test]
    fn tiled_pitch_above_max_returns_zero() {
        assert_eq!(tiled_pitch_lookup(0x10001), 0);
    }

    #[test]
    fn tiled_pitches_table_is_sorted() {
        use crate::hle::cell_gcm_sys::TILED_PITCHES;
        for w in TILED_PITCHES.windows(2) {
            assert!(w[0] < w[1], "table not sorted: {} >= {}", w[0], w[1]);
        }
    }

    #[test]
    fn hle_module_nid_sets_are_disjoint() {
        use std::collections::BTreeSet;
        let modules: &[(&str, &[u32])] = &[
            ("sys_prx_for_user", crate::hle::sys_prx_for_user::OWNED_NIDS),
            ("cell_gcm_sys", crate::hle::cell_gcm_sys::OWNED_NIDS),
            ("cell_spurs", crate::hle::cell_spurs::OWNED_NIDS),
        ];
        let mut all = BTreeSet::new();
        for (name, nids) in modules {
            let set: BTreeSet<u32> = nids.iter().copied().collect();
            assert_eq!(
                set.len(),
                nids.len(),
                "hle::{name}::OWNED_NIDS contains duplicates"
            );
            for nid in &set {
                assert!(
                    all.insert(*nid),
                    "HLE module NID collision: {nid:#010x} appears in {name} and another module"
                );
            }
        }
    }

    /// Pins the order: counter `+=` must precede the `debug_assert!`
    /// so debug and release populate the same map.
    #[test]
    fn adapter_drop_bumps_handlers_without_mutation_counter() {
        use crate::hle::context::RuntimeHleAdapter;
        use cellgov_mem::GuestMemory;
        use std::collections::BTreeMap;

        let mut memory = GuestMemory::new(0x10000);
        let mut registry = crate::registry::UnitRegistry::new();
        registry.register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let mut heap_ptr: u32 = 0x1000;
        let mut heap_watermark: u32 = 0x1000;
        let mut heap_warning_mask: u8 = 0;
        let mut next_id: u32 = 0x8000_0001;
        let mut counter: BTreeMap<u32, usize> = BTreeMap::new();

        let probe_nid: u32 = 0xBADF_00D0;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _adapter = RuntimeHleAdapter {
                memory: &mut memory,
                registry: &mut registry,
                heap_base: 0x1000,
                heap_ptr: &mut heap_ptr,
                heap_watermark: &mut heap_watermark,
                heap_warning_mask: &mut heap_warning_mask,
                next_id: &mut next_id,
                source: cellgov_event::UnitId::new(0),
                nid: probe_nid,
                mutated: false,
                handlers_without_mutation: &mut counter,
            };
        }));

        assert_eq!(
            counter.get(&probe_nid),
            Some(&1),
            "Drop guard should bump handlers_without_mutation[{probe_nid:#010x}] \
             regardless of debug/release mode; got {:?}",
            counter
        );
    }

    #[test]
    fn gcm_init_body_writes_context_and_callback() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::INIT_BODY, &args);

        let mem = rt.memory().as_bytes();
        let ctx_ptr = u32::from_be_bytes([mem[0x10000], mem[0x10001], mem[0x10002], mem[0x10003]]);
        assert_ne!(ctx_ptr, 0, "context pointer should be non-zero");

        let a = ctx_ptr as usize;
        let begin = u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]]);
        let end = u32::from_be_bytes([mem[a + 4], mem[a + 5], mem[a + 6], mem[a + 7]]);
        let callback = u32::from_be_bytes([mem[a + 12], mem[a + 13], mem[a + 14], mem[a + 15]]);
        assert_eq!(begin, 0x20000 + 0x1000, "begin = ioAddress + 0x1000");
        assert!(end > begin, "end > begin");
        assert_ne!(callback, 0, "callback OPD should be non-zero");

        assert_eq!(rt.hle.gcm.control_addr, 0xC000_0040);
    }

    #[test]
    fn gcm_init_body_without_rsx_checkpoint_forwards_to_sys_rsx() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x4000_0000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x10_0000);
        rt.set_gcm_rsx_checkpoint(false);

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::INIT_BODY, &args);

        let base = cellgov_lv2::host::Lv2Host::SYS_RSX_MEM_BASE;
        assert_eq!(rt.hle.gcm.control_addr, base + 0x40);
        assert_eq!(rt.hle.gcm.label_addr, base + 0x20_0000);

        // Label 255 reads the LV2 sentinel written into the reports
        // region at sys_rsx context allocation.
        let label_addr = rt.hle.gcm.label_addr as usize;
        let sentinel_offset = label_addr + 255 * 0x10;
        let mem = rt.memory().as_bytes();
        let sentinel = u32::from_be_bytes([
            mem[sentinel_offset],
            mem[sentinel_offset + 1],
            mem[sentinel_offset + 2],
            mem[sentinel_offset + 3],
        ]);
        assert_eq!(sentinel, 0x1337_C0D3);
    }

    /// Pins that both `init_body` branches produce a non-zero
    /// `control_addr`, which the GET_CONTROL_REGISTER dispatch
    /// `debug_assert_ne!` relies on as its "init ran" witness.
    #[test]
    fn gcm_init_body_control_addr_is_nonzero_in_both_modes() {
        use crate::hle::cell_gcm_sys::{init_body, GcmState};
        use crate::hle::context::RuntimeHleAdapter;
        use cellgov_mem::GuestMemory;

        fn run_init(rsx_checkpoint: bool) -> u32 {
            let mut memory = GuestMemory::new(0x200000);
            let mut registry = crate::registry::UnitRegistry::new();
            registry.register_with(|id| {
                cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
            });
            let mut heap_ptr: u32 = 0x100000;
            let mut heap_watermark: u32 = 0x100000;
            let mut heap_warning_mask: u8 = 0;
            let mut next_id: u32 = 0x8000_0001;
            let mut handlers_without_mutation: std::collections::BTreeMap<u32, usize> =
                std::collections::BTreeMap::new();
            let mut gcm = GcmState {
                rsx_checkpoint,
                ..Default::default()
            };
            let mut ctx = RuntimeHleAdapter {
                memory: &mut memory,
                registry: &mut registry,
                heap_base: 0x100000,
                heap_ptr: &mut heap_ptr,
                heap_watermark: &mut heap_watermark,
                heap_warning_mask: &mut heap_warning_mask,
                next_id: &mut next_id,
                source: cellgov_event::UnitId::new(0),
                nid: gcm_nid::INIT_BODY,
                mutated: false,
                handlers_without_mutation: &mut handlers_without_mutation,
            };
            let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
            init_body(&mut ctx, &args, &mut gcm);
            gcm.control_addr
        }

        let ctrl_with_checkpoint = run_init(true);
        assert_eq!(
            ctrl_with_checkpoint, 0xC000_0040,
            "rsx_checkpoint mode must pin control_addr to the MMIO sentinel; \
             if you changed this value update the dispatch witness too"
        );
        assert_ne!(
            ctrl_with_checkpoint, 0,
            "control_addr sentinel cannot be 0 (dispatch witness would silently pass pre-init)"
        );

        let ctrl_no_checkpoint = run_init(false);
        assert_ne!(
            ctrl_no_checkpoint, 0,
            "non-checkpoint mode must heap-allocate a non-zero control_addr \
             (dispatch witness relies on both init branches producing non-zero)"
        );
    }

    #[test]
    fn gcm_set_flip_handler_records_callback_address() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        assert_eq!(rt.rsx_flip().handler(), 0, "starts cleared");
        let args: [u64; 9] = [0, 0x1234_5678, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::SET_FLIP_HANDLER, &args);

        assert_eq!(rt.rsx_flip().handler(), 0x1234_5678);
        assert_eq!(
            rt.rsx_flip().status(),
            crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_DONE
        );
        assert!(!rt.rsx_flip().pending());
    }

    #[test]
    fn gcm_set_flip_handler_forwards_to_sys_rsx_when_allocated() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x4000_0000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x10_0000);
        rt.set_gcm_rsx_checkpoint(false);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::INIT_BODY, &args);
        assert!(rt.lv2_host().sys_rsx_context().allocated);

        let handler: u32 = 0x1234_5678;
        let args: [u64; 9] = [0, handler as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::SET_FLIP_HANDLER, &args);
        assert_eq!(rt.rsx_flip().handler(), handler);
        assert_eq!(
            rt.lv2_host().sys_rsx_context().flip_handler_addr,
            handler,
            "sys_rsx authoritative state should mirror the handler address"
        );
    }

    #[test]
    fn gcm_set_flip_handler_accepts_null_to_clear() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);
        rt.rsx_flip_mut().set_handler(0xAABB_CCDD);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0; 9];
        rt.dispatch_hle(unit_id, gcm_nid::SET_FLIP_HANDLER, &args);

        assert_eq!(rt.rsx_flip().handler(), 0, "NULL cleared the handler");
    }

    #[test]
    fn gcm_init_body_leaves_labels_zero_initialised() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::INIT_BODY, &args);

        let label_addr = rt.hle.gcm.label_addr;
        assert_ne!(label_addr, 0, "init must have allocated a label region");
        let mem = rt.memory().as_bytes();
        let base = label_addr as usize;
        for (i, byte) in mem[base..base + 4096].iter().enumerate() {
            assert_eq!(
                *byte, 0,
                "label byte at offset {i:#x} must be zero post-init; got {byte:#x}"
            );
        }
    }

    #[test]
    fn gcm_get_configuration_writes_config() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);
        rt.hle.gcm.io_address = 0x20000;
        rt.hle.gcm.io_size = 0x80000;
        rt.hle.gcm.local_size = 0x0f90_0000;
        // Satisfy the `context_addr != 0` "init ran" witness.
        rt.hle.gcm.context_addr = 0x101000;

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x10000, 0x10000, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::GET_CONFIGURATION, &args);

        let mem = rt.memory().as_bytes();
        let a = 0x10000usize;
        let local_addr = u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]]);
        let io_addr = u32::from_be_bytes([mem[a + 4], mem[a + 5], mem[a + 6], mem[a + 7]]);
        let local_size = u32::from_be_bytes([mem[a + 8], mem[a + 9], mem[a + 10], mem[a + 11]]);
        let io_size = u32::from_be_bytes([mem[a + 12], mem[a + 13], mem[a + 14], mem[a + 15]]);
        assert_eq!(local_addr, 0xC000_0000);
        assert_eq!(io_addr, 0x20000);
        assert_eq!(local_size, 0x0f90_0000);
        assert_eq!(io_size, 0x80000);
    }

    #[test]
    fn gcm_get_label_address_returns_indexed_offset() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_gcm_rsx_checkpoint(true);
        // Satisfy the `label_addr != 0` "init ran" witness.
        rt.hle.gcm.label_addr = 0x50000;

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args0: [u64; 9] = [0x10000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::GET_LABEL_ADDRESS, &args0);
        let ret0 = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret0, Some(0x50000));

        let args5: [u64; 9] = [0x10000, 5, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, gcm_nid::GET_LABEL_ADDRESS, &args5);
        let ret5 = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret5, Some(0x50000 + 5 * 0x10));
    }

    #[test]
    fn sys_ppu_thread_get_id_returns_primary_when_unit_seeded_in_table() {
        // Route through the PpuThreadTable lookup, not the
        // 0x0100_0000 fallback. The two produce the same value for
        // the primary thread; this test pins the routing so child
        // threads pick up their minted ids through the same path.
        use crate::runtime::Runtime;
        use cellgov_lv2::PpuThreadAttrs;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x100000), Budget::new(1), 100);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let attrs = PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut().seed_primary_ppu_thread(unit_id, attrs);

        let args: [u64; 9] = [0x2000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_nid::PPU_THREAD_GET_ID, &args);

        let mem = rt.memory().as_bytes();
        let tid = u64::from_be_bytes([
            mem[0x2000],
            mem[0x2001],
            mem[0x2002],
            mem[0x2003],
            mem[0x2004],
            mem[0x2005],
            mem[0x2006],
            mem[0x2007],
        ]);
        assert_eq!(tid, 0x0100_0000);
        assert_eq!(rt.registry_mut().drain_syscall_return(unit_id), Some(0));
    }

    #[test]
    fn sys_ppu_thread_join_on_finished_target_writes_exit_value() {
        use crate::runtime::Runtime;
        use cellgov_lv2::{Lv2Request, PpuThreadAttrs};
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let primary = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let attrs = || PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut().seed_primary_ppu_thread(primary, attrs());
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let child_id = rt
            .lv2_host_mut()
            .ppu_threads_mut()
            .create(cellgov_event::UnitId::new(1), attrs())
            .expect("child create");
        rt.lv2_host_mut()
            .ppu_threads_mut()
            .mark_finished(child_id, 0xCAFE_F00D);

        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadJoin {
                target: child_id.raw(),
                status_out_ptr: 0x5000,
            },
            primary,
        );

        assert_eq!(rt.registry_mut().drain_syscall_return(primary), Some(0));
        let mem = rt.memory().as_bytes();
        let status = u64::from_be_bytes([
            mem[0x5000],
            mem[0x5001],
            mem[0x5002],
            mem[0x5003],
            mem[0x5004],
            mem[0x5005],
            mem[0x5006],
            mem[0x5007],
        ]);
        assert_eq!(status, 0xCAFE_F00D);
    }

    #[test]
    fn sys_ppu_thread_join_blocks_then_wakes_with_exit_value() {
        use crate::runtime::Runtime;
        use cellgov_lv2::{Lv2Request, PpuThreadAttrs};
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let primary = cellgov_event::UnitId::new(0);
        let child = cellgov_event::UnitId::new(1);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let attrs = || PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut().seed_primary_ppu_thread(primary, attrs());
        let child_id = rt
            .lv2_host_mut()
            .ppu_threads_mut()
            .create(child, attrs())
            .expect("child create");

        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadJoin {
                target: child_id.raw(),
                status_out_ptr: 0x5000,
            },
            primary,
        );
        assert_eq!(
            rt.registry().effective_status(primary),
            Some(cellgov_exec::UnitStatus::Blocked),
        );

        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadExit {
                exit_value: 0xABCD_1234,
            },
            child,
        );

        assert_eq!(
            rt.registry().effective_status(primary),
            Some(cellgov_exec::UnitStatus::Runnable),
        );
        assert_eq!(rt.registry_mut().drain_syscall_return(primary), Some(0));
        let mem = rt.memory().as_bytes();
        let status = u64::from_be_bytes([
            mem[0x5000],
            mem[0x5001],
            mem[0x5002],
            mem[0x5003],
            mem[0x5004],
            mem[0x5005],
            mem[0x5006],
            mem[0x5007],
        ]);
        assert_eq!(status, 0xABCD_1234);
    }

    #[test]
    fn sys_ppu_thread_create_registers_child_unit_and_mints_thread_id() {
        use crate::runtime::Runtime;
        use cellgov_lv2::{Lv2Request, PpuThreadAttrs};
        use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let primary = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let primary_attrs = PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut()
            .seed_primary_ppu_thread(primary, primary_attrs);

        use std::cell::Cell;
        use std::rc::Rc;
        let calls = Rc::new(Cell::new(0u32));
        let calls_clone = calls.clone();
        rt.set_ppu_factory(move |id, _init| {
            calls_clone.set(calls_clone.get() + 1);
            Box::new(cellgov_exec::FakeIsaUnit::new(
                id,
                vec![cellgov_exec::FakeOp::End],
            ))
        });

        let mut opd = [0u8; 16];
        opd[0..8].copy_from_slice(&0x4_0000u64.to_be_bytes());
        opd[8..16].copy_from_slice(&0x5_0000u64.to_be_bytes());
        rt.memory_mut()
            .apply_commit(ByteRange::new(GuestAddr::new(0x2_0000), 16).unwrap(), &opd)
            .unwrap();

        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x3_0000,
                entry_opd: 0x2_0000,
                arg: 0xCAFE_BABE,
                priority: 1500,
                stacksize: 0x10_000,
                flags: 0,
            },
            primary,
        );

        assert_eq!(calls.get(), 1);
        let ids: Vec<_> = rt.registry().ids().collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(rt.registry_mut().drain_syscall_return(primary), Some(0));
        let child_unit_id = ids[1];
        let child_thread = rt
            .lv2_host()
            .ppu_thread_for_unit(child_unit_id)
            .expect("child in thread table");
        assert!(child_thread.id.raw() > cellgov_lv2::PpuThreadId::PRIMARY.raw());
        let mem = rt.memory().as_bytes();
        let written = u64::from_be_bytes([
            mem[0x3_0000],
            mem[0x3_0001],
            mem[0x3_0002],
            mem[0x3_0003],
            mem[0x3_0004],
            mem[0x3_0005],
            mem[0x3_0006],
            mem[0x3_0007],
        ]);
        assert_eq!(written, child_thread.id.raw());
    }

    #[test]
    fn sys_ppu_thread_get_id_returns_child_id_when_child_registered() {
        use crate::runtime::Runtime;
        use cellgov_lv2::PpuThreadAttrs;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x100000), Budget::new(1), 100);
        let primary = cellgov_event::UnitId::new(0);
        let child = cellgov_event::UnitId::new(1);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let attrs = || PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut().seed_primary_ppu_thread(primary, attrs());
        let child_id = rt
            .lv2_host_mut()
            .ppu_threads_mut()
            .create(child, attrs())
            .expect("child create");
        assert_ne!(child_id.raw(), 0x0100_0000);

        let args: [u64; 9] = [0x3000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(child, sys_nid::PPU_THREAD_GET_ID, &args);

        let mem = rt.memory().as_bytes();
        let tid = u64::from_be_bytes([
            mem[0x3000],
            mem[0x3001],
            mem[0x3002],
            mem[0x3003],
            mem[0x3004],
            mem[0x3005],
            mem[0x3006],
            mem[0x3007],
        ]);
        assert_eq!(tid, child_id.raw());
    }

    #[test]
    fn sys_time_get_system_time_returns_microseconds_from_guest_clock() {
        // Microseconds since boot, derived from `runtime.time()`
        // (1 tick = 1 ns -> us = ticks / 1000). At t=0 the call
        // returns 0; advancing the guest clock advances the result.
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x100000), Budget::new(1), 100);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0; 9];
        rt.dispatch_hle(unit_id, sys_nid::TIME_GET_SYSTEM_TIME, &args);
        assert_eq!(rt.registry_mut().drain_syscall_return(unit_id), Some(0));
    }
}
