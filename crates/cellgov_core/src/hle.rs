//! HLE (High-Level Emulation) router.
//!
//! Each PS3 system library is one file in this directory whose
//! filename matches the canonical Sony library name
//! (`cellGcmSys.rs`, `sysPrxForUser.rs`, and so on) so a visual
//! grep against RPCS3's source tree lines up one-to-one. Each file
//! owns its NID constants, its handler bodies, and a `dispatch`
//! entry point returning `Option<()>`. This file chains those
//! entry points; the first module to claim a NID wins, and
//! unclaimed NIDs fall through to the CELL_OK default.
//!
//! Rust module identifiers stay snake_case (`cell_gcm_sys`,
//! `sys_prx_for_user`) because Rust's `non_snake_case` lint would
//! fire otherwise at every call site; the `#[path]` attribute on
//! each `mod` declaration below bridges the two. Reading code and
//! reading Sony docs both work without either fighting the other.
//!
//! Scaling rule: a new PS3 library means a new
//! `crates/cellgov_core/src/hle/<CanonicalName>.rs` file, one
//! `#[path]`-prefixed `pub mod` declaration below, and one line
//! added to the chain. No registration macro, no mutable global
//! dispatch table, no link-time wiring.
//!
//! ## Unclaimed-NID policy
//!
//! When no module claims a NID, the router sets r3 = CELL_OK (0)
//! and emits no side effects. This is the only *safe* default --
//! every PS3 library function is also the site of a silent divergence
//! if it was genuinely called and needed real behavior. The router
//! records every unclaimed call (see `HleState::unclaimed_nids`
//! and [`Runtime::hle_unclaimed_nids`]) and prints a one-shot
//! stderr line the first time each NID shows up so "no one
//! implemented this" stays distinguishable from "this was called
//! and worked." Downstream observability (divergence localization,
//! run-game summary) can walk the counter to attribute corrupted
//! state to a specific unimplemented library entry.

use cellgov_event::UnitId;

use crate::hle::cell_gcm_sys::GcmState;
use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

// Filenames match the canonical Sony library names (cellGcmSys,
// sysPrxForUser) so a visual grep against RPCS3's source tree
// lines up one-to-one. Rust module identifiers stay snake_case
// so call sites do not fight the `non_snake_case` lint; the
// `#[path]` attribute decouples the two.
#[path = "hle/cellGcmSys.rs"]
pub(crate) mod cell_gcm_sys;
pub mod context;
#[path = "hle/sysPrxForUser.rs"]
pub(crate) mod sys_prx_for_user;

/// HLE-specific bookkeeping bundled off the `Runtime` struct.
///
/// Groups the HLE fields so the `Runtime` struct definition reads
/// as orchestration-only state. HLE handlers receive a
/// `&mut HleState` via split-borrow from `Runtime`; none of these
/// fields has meaning outside the HLE dispatch path.
pub(crate) struct HleState {
    /// HLE NID table: maps HLE index -> NID for dispatch of HLE
    /// calls that need non-trivial behavior (e.g., TLS init,
    /// mutex create).
    pub nids: std::collections::BTreeMap<u32, u32>,
    /// Base the bump allocator started from (the most recent value
    /// passed to `Runtime::set_hle_heap_base`). Tracked alongside
    /// `heap_ptr` so the heap-watermark threshold check knows how
    /// many bytes have actually been handed out rather than looking
    /// at the raw cursor (which starts nonzero).
    pub heap_base: u32,
    /// Bump allocator pointer for `_sys_malloc` HLE. Points to the
    /// next free address in guest memory. Allocations are never
    /// freed.
    pub heap_ptr: u32,
    /// Peak value `heap_ptr` has ever reached. Diagnostic for
    /// deciding whether the bump allocator's leak-on-free policy
    /// needs to become a real free-list allocator. See
    /// `hle::sys_prx_for_user::NID_SYS_FREE` for the TODO and the design
    /// sketch. The threshold check in `heap_alloc` emits a
    /// one-shot stderr warning every time `heap_watermark -
    /// heap_base` crosses 1 MiB / 10 MiB / 100 MiB so the signal
    /// is visible without grepping output.
    pub heap_watermark: u32,
    /// Bitmask of which heap-watermark bands have already been
    /// reported. Bits: 0x1 = 1 MiB, 0x2 = 10 MiB, 0x4 = 100 MiB.
    /// Kept next to the watermark so the threshold check in
    /// `heap_alloc` is truly one-shot even if a handler is called
    /// repeatedly.
    pub heap_warning_mask: u8,
    /// Monotonic kernel-object ID counter for HLE-created primitives
    /// (lwmutex sleep_queue, etc.). Starts above zero so a zero-
    /// initialized guest field is distinguishable from a legitimate
    /// allocated ID.
    pub next_id: u32,
    /// cellGcmSys state (io/local region sizes, control/label
    /// addresses, RSX-checkpoint toggle).
    pub gcm: GcmState,
    /// Count of dispatched NIDs that no module claimed. Keyed by
    /// NID so a run-game summary can surface every unimplemented
    /// library entry the scenario touched. First occurrence of each
    /// NID also emits a one-shot stderr line in `dispatch_hle`.
    pub unclaimed_nids: std::collections::BTreeMap<u32, usize>,
    /// Count of NIDs that *were* claimed by a module and routed to
    /// a handler but produced no observable mutation before the
    /// adapter went out of scope. Populated from the `Drop` impl of
    /// [`context::RuntimeHleAdapter`] in both debug and release
    /// builds. Same shape as `unclaimed_nids` (BTreeMap keyed by
    /// NID), different population: a non-zero entry here is a
    /// handler whose register state leaked straight through to the
    /// guest without a fresh r3, which is a silent-divergence
    /// footgun the debug-only `debug_assert!` would catch in tests
    /// but the release build would otherwise hide.
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
    /// Walks the module chain in priority order. If no module
    /// claims the NID, the default is `r3 = 0` (CELL_OK) with no
    /// effects -- the safe fallback for unobserved calls, paired
    /// with an observability trail so downstream analysis can tell
    /// "not implemented" apart from "worked." See the module doc
    /// comment's "Unclaimed-NID policy" section.
    pub(crate) fn dispatch_hle(&mut self, source: UnitId, nid: u32, args: &[u64; 9]) {
        let handled = sys_prx_for_user::dispatch(self, source, nid, args)
            .or_else(|| cell_gcm_sys::dispatch(self, source, nid, args));
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
    use crate::hle::cell_gcm_sys::{
        NID_CELLGCM_GET_CONFIGURATION, NID_CELLGCM_GET_LABEL_ADDRESS, NID_CELLGCM_INIT_BODY,
    };
    use crate::hle::sys_prx_for_user::{NID_SYS_PPU_THREAD_GET_ID, NID_SYS_TIME_GET_SYSTEM_TIME};

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

    /// Window-boundary canary: `0x201` sits just past the first
    /// window `(0x000, 0x200]` and must fall into `(0x200, 0x300]`,
    /// returning `0x300`. Cross-validate against RPCS3's
    /// `cellGcmGetTiledPitchSize` if the table or the window
    /// predicate ever changes.
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

    /// Disjointness contract for the HLE dispatch chain. `dispatch_hle`
    /// walks modules in priority order (`sys` then `gcm`) and the
    /// first to claim a NID wins; a NID defined in both modules
    /// silently masks the later handler. Each module exports an
    /// `OWNED_NIDS` const slice naming every NID it dispatches on;
    /// this test asserts the union is disjoint so a refactor that
    /// accidentally duplicates a NID fails CI instead of producing
    /// a ghost handler in production.
    #[test]
    fn hle_module_nid_sets_are_disjoint() {
        use std::collections::BTreeSet;
        let sys: BTreeSet<u32> = crate::hle::sys_prx_for_user::OWNED_NIDS
            .iter()
            .copied()
            .collect();
        let gcm: BTreeSet<u32> = crate::hle::cell_gcm_sys::OWNED_NIDS
            .iter()
            .copied()
            .collect();
        let overlap: Vec<u32> = sys.intersection(&gcm).copied().collect();
        assert!(
            overlap.is_empty(),
            "HLE module NID collision: {:#010x?} owned by both sys and gcm",
            overlap
        );
        // Also pin that each module's OWNED_NIDS has no internal
        // duplicates -- a duplicate entry would silently mask the
        // disjointness check above.
        assert_eq!(
            sys.len(),
            crate::hle::sys_prx_for_user::OWNED_NIDS.len(),
            "hle::sys_prx_for_user::OWNED_NIDS contains duplicates"
        );
        assert_eq!(
            gcm.len(),
            crate::hle::cell_gcm_sys::OWNED_NIDS.len(),
            "hle::cell_gcm_sys::OWNED_NIDS contains duplicates"
        );
    }

    /// The Drop guard on `RuntimeHleAdapter` bumps
    /// `HleState::handlers_without_mutation[nid]` when a handler
    /// constructs an adapter but returns without touching any
    /// state. Debug builds then `debug_assert!(false, ...)` to
    /// fail loudly in CI; release builds skip the panic so the
    /// counter becomes the production signal.
    ///
    /// This test verifies the counter is bumped even in debug
    /// builds (we catch the debug_assert panic). The order matters:
    /// the counter `+=` must happen before the `debug_assert!` so
    /// both modes populate the same map. If a future refactor
    /// reorders them, the assertion fires here.
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

        // Scope the adapter so its Drop fires inside catch_unwind.
        // In debug, the Drop impl panics AFTER bumping the counter;
        // in release, no panic. catch_unwind captures either outcome
        // without distinguishing them at the test level.
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
            // Dropped at end of scope without any mutating call --
            // exactly the silent-divergence footgun the counter
            // exists to catch.
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
        rt.dispatch_hle(unit_id, NID_CELLGCM_INIT_BODY, &args);

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
        // With rsx_checkpoint off, cellGcmInitBody forwards to
        // sys_rsx_memory_allocate + sys_rsx_context_allocate and the
        // resulting control / label addresses come from Lv2Host's
        // SysRsxContext rather than the HLE heap.
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
        rt.dispatch_hle(unit_id, NID_CELLGCM_INIT_BODY, &args);

        // Addresses match the sys_rsx reservation layout: dma_control
        // at SYS_RSX_MEM_BASE, control_addr at +0x40 (past the reserved
        // prefix), reports at SYS_RSX_MEM_BASE + 0x200000.
        let base = cellgov_lv2::host::Lv2Host::SYS_RSX_MEM_BASE;
        assert_eq!(rt.hle.gcm.control_addr, base + 0x40);
        assert_eq!(rt.hle.gcm.label_addr, base + 0x20_0000);

        // Label 255 reads 0x1337_C0D3 -- the LV2 sentinel written
        // into the reports region at sys_rsx context allocation.
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

    /// Canary: the GET_CONTROL_REGISTER dispatch in gcm.rs uses
    /// `debug_assert_ne!(control_addr, 0, ...)` as its "init ran"
    /// witness. The witness relies on BOTH `init_body` branches
    /// producing a non-zero `control_addr` -- `0xC000_0040` when
    /// `rsx_checkpoint` is on, and a non-zero heap allocation
    /// otherwise. The dispatch path itself currently gates on
    /// `rsx_checkpoint` being true, so the `else` branch is not
    /// reachable via `dispatch_hle` today; this test calls
    /// `init_body` directly to cover it anyway, because (a) the
    /// code exists and could be reached if the dispatch gate
    /// changes, and (b) the policy we are pinning is "both init
    /// paths produce a non-zero witness." If you are reading
    /// this because you changed either the MMIO base literal,
    /// the fallback allocator, or the dispatch gate, also
    /// update the witness and the `GcmState::control_addr`
    /// comment to match.
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
                nid: crate::hle::cell_gcm_sys::NID_CELLGCM_INIT_BODY,
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
        // cellGcmSetFlipHandler records the callback pointer into
        // RsxFlipState::handler without touching any other flip
        // field. Returns 0 (Sony's signature is void; 0 is the
        // CELL_OK coercion).
        use crate::hle::cell_gcm_sys::NID_CELLGCM_SET_FLIP_HANDLER;
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
        rt.dispatch_hle(unit_id, NID_CELLGCM_SET_FLIP_HANDLER, &args);

        assert_eq!(rt.rsx_flip().handler(), 0x1234_5678);
        // Other fields unchanged.
        assert_eq!(
            rt.rsx_flip().status(),
            crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_DONE
        );
        assert!(!rt.rsx_flip().pending());
    }

    #[test]
    fn gcm_set_flip_handler_forwards_to_sys_rsx_when_allocated() {
        use crate::hle::cell_gcm_sys::NID_CELLGCM_SET_FLIP_HANDLER;
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

        // Allocate the context via cellGcmInitBody's sys_rsx path.
        let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            crate::hle::cell_gcm_sys::NID_CELLGCM_INIT_BODY,
            &args,
        );
        assert!(rt.lv2_host().sys_rsx_context().allocated);

        // Register a flip handler; check it lands in both places.
        let handler: u32 = 0x1234_5678;
        let args: [u64; 9] = [0, handler as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELLGCM_SET_FLIP_HANDLER, &args);
        assert_eq!(rt.rsx_flip().handler(), handler);
        assert_eq!(
            rt.lv2_host().sys_rsx_context().flip_handler_addr,
            handler,
            "sys_rsx authoritative state should mirror the handler address"
        );
    }

    #[test]
    fn gcm_set_flip_handler_accepts_null_to_clear() {
        // Games passing NULL to clear the handler is a legal call
        // pattern. Verify the oracle records 0 without error.
        use crate::hle::cell_gcm_sys::NID_CELLGCM_SET_FLIP_HANDLER;
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);
        // Pre-seed a handler so the clear is visible.
        rt.rsx_flip_mut().set_handler(0xAABB_CCDD);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0; 9];
        rt.dispatch_hle(unit_id, NID_CELLGCM_SET_FLIP_HANDLER, &args);

        assert_eq!(rt.rsx_flip().handler(), 0, "NULL cleared the handler");
    }

    #[test]
    fn gcm_init_body_leaves_labels_zero_initialised() {
        // Post-init every byte in the label region must read as 0
        // so the FIFO advance pass is the sole source of label
        // values. A guest polling a label for a specific non-zero
        // value correctly spins until a method writes it.
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
        rt.dispatch_hle(unit_id, NID_CELLGCM_INIT_BODY, &args);

        let label_addr = rt.hle.gcm.label_addr;
        assert_ne!(label_addr, 0, "init must have allocated a label region");
        let mem = rt.memory().as_bytes();
        let base = label_addr as usize;
        // Sample the whole region. Any non-zero byte is a 0xFF
        // pre-fill leaking back in; the sample is cheap so we
        // check the full 4K window rather than a prefix.
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
        // Simulate post-init state: the GetConfiguration handler
        // guards on context_addr != 0 as its structural "init
        // already ran" witness.
        rt.hle.gcm.context_addr = 0x101000;

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x10000, 0x10000, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELLGCM_GET_CONFIGURATION, &args);

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
        // GetLabelAddress guards on label_addr != 0 as its "init
        // already ran" witness; setting label_addr here satisfies
        // both the handler's invariant and the test's intent.
        rt.hle.gcm.label_addr = 0x50000;

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args0: [u64; 9] = [0x10000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELLGCM_GET_LABEL_ADDRESS, &args0);
        let ret0 = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret0, Some(0x50000));

        let args5: [u64; 9] = [0x10000, 5, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELLGCM_GET_LABEL_ADDRESS, &args5);
        let ret5 = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret5, Some(0x50000 + 5 * 0x10));
    }

    // (The former `sys_ppu_thread_get_id_writes_thread_id_and_returns_zero`
    // test used to pin the unseeded-LV2-table fallback behavior as if
    // it were a contract. Real boots always seed the primary thread
    // via `seed_primary_ppu_thread` before guest code runs -- see
    // `apps/cellgov_cli/src/game/boot.rs`. The fallback in
    // `hle::sys_prx_for_user` now has a `debug_assert!` that fires on unseeded
    // dispatches, so that test became self-contradictory. The
    // `sys_ppu_thread_get_id_returns_primary_when_unit_seeded_in_table`
    // test below covers the real path.)

    #[test]
    fn sys_ppu_thread_get_id_returns_primary_when_unit_seeded_in_table() {
        // Once the primary thread is recorded in the
        // PpuThreadTable, get_id must route through the table
        // lookup rather than the 0x0100_0000 fallback. The two
        // paths happen to produce the same value for the primary
        // thread, but this test nails the routing so that child
        // threads created via sys_ppu_thread_create pick up
        // their minted ids through the same path.
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
        rt.dispatch_hle(unit_id, NID_SYS_PPU_THREAD_GET_ID, &args);

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
        rt.dispatch_hle(child, NID_SYS_PPU_THREAD_GET_ID, &args);

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
    fn sys_time_get_system_time_returns_nonzero_monotonic() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x100000), Budget::new(1), 100);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0; 9];
        rt.dispatch_hle(unit_id, NID_SYS_TIME_GET_SYSTEM_TIME, &args);

        let ret = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret, Some(1_000_000));
    }
}
