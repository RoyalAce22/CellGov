//! sysPrxForUser HLE implementations.
//!
//! Covers the userspace PRX surface exposed as sysPrxForUser (and
//! the RPCS3 sub-files `sys_lwmutex_.cpp`, `sys_ppu_thread_.cpp`,
//! `sys_heap.cpp`, `sys_libc.cpp`, `sys_prx_.cpp` that register
//! into the same module). Kernel-side syscalls (`sc` trap
//! handlers) live in `cellgov_lv2`, not here.
//!
//! The module exports a single [`dispatch`] entry that the HLE
//! router in [`crate::hle`] chains via `.or_else()`. Each module
//! file follows the same shape so adding a new library is an
//! additive change to the router and a new file, never a shared
//! match statement.
//!
//! Handler NID constants are module-local. The workspace name
//! database (`cellgov_ppu::nid_db`) is the global reverse lookup
//! for diagnostics and tracing, distinct from dispatch.
//!
//! ## Failure policy
//!
//! Same framework as [`crate::hle::cell_gcm_sys`]: fidelity to real PS3
//! beats defensiveness.
//!
//! - Invariants that only our loader/runtime can violate (malformed
//!   TLS from the ELF loader, unseeded LV2 thread table) use
//!   `debug_assert!` so tests surface oracle bugs while release
//!   matches historical fallback behavior.
//! - Guest-supplied bad pointers return `CELL_EFAULT` (0x8001000e)
//!   to match RPCS3's `vm::ptr` auto-validation semantics, rather
//!   than silently skipping the side effect and reporting success.
//! - `.expect(...)` stays reserved for oracle-state corruption
//!   (heap exhaustion, ID counter exhaustion).

use cellgov_event::UnitId;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

/// `CELL_EFAULT`: guest supplied a pointer that could not be
/// dereferenced. Matches the PS3 error constant; also referenced
/// by `runtime/ppu_create.rs` which returns the same value from
/// sys_ppu_thread_create on bad OPD.
const CELL_EFAULT: u64 = 0x8001_000e;

pub(crate) const NID_SYS_INITIALIZE_TLS: u32 = 0x744680a2;
pub(crate) const NID_SYS_PROCESS_EXIT: u32 = 0xe6f2c1e7;
pub(crate) const NID_SYS_MALLOC: u32 = 0xbdb18f83;
pub(crate) const NID_SYS_FREE: u32 = 0xf7f7fb20;
pub(crate) const NID_SYS_MEMSET: u32 = 0x68b9b011;
pub(crate) const NID_SYS_LWMUTEX_CREATE: u32 = 0x2f85c0ef;
pub(crate) const NID_SYS_HEAP_CREATE_HEAP: u32 = 0xb2fcf2c8;
pub(crate) const NID_SYS_HEAP_DELETE_HEAP: u32 = 0xaede4b03;
pub(crate) const NID_SYS_HEAP_MALLOC: u32 = 0x35168520;
pub(crate) const NID_SYS_HEAP_MEMALIGN: u32 = 0x44265c08;
pub(crate) const NID_SYS_HEAP_FREE: u32 = 0x8a561d92;
pub(crate) const NID_SYS_PPU_THREAD_GET_ID: u32 = 0x350d454e;
pub(crate) const NID_SYS_THREAD_CREATE_EX: u32 = 0x24a1ea07;
pub(crate) const NID_SYS_THREAD_EXIT: u32 = 0xaff080a4;
pub(crate) const NID_SYS_TIME_GET_SYSTEM_TIME: u32 = 0x8461e528;

/// Every NID this module claims. Consumed by two test-only canaries
/// in `crate::hle::tests` and the module-local `canary_tests` block:
/// (a) the disjointness test asserts no NID overlaps with another
/// module's list, (b) the dispatch-coverage test asserts every NID
/// here is actually claimed by [`dispatch`]. The reverse drift
/// (new `match` arm whose NID is absent from this list) is surfaced
/// operator-side via [`crate::Runtime::hle_unclaimed_nids`].
#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = &[
    NID_SYS_INITIALIZE_TLS,
    NID_SYS_PROCESS_EXIT,
    NID_SYS_MALLOC,
    NID_SYS_FREE,
    NID_SYS_MEMSET,
    NID_SYS_LWMUTEX_CREATE,
    NID_SYS_HEAP_CREATE_HEAP,
    NID_SYS_HEAP_DELETE_HEAP,
    NID_SYS_HEAP_MALLOC,
    NID_SYS_HEAP_MEMALIGN,
    NID_SYS_HEAP_FREE,
    NID_SYS_PPU_THREAD_GET_ID,
    NID_SYS_THREAD_CREATE_EX,
    NID_SYS_THREAD_EXIT,
    NID_SYS_TIME_GET_SYSTEM_TIME,
];

/// Dispatch entry point for sysPrxForUser handlers.
///
/// Returns `Some(())` if the NID was handled (the handler has
/// already written r3 and any out-pointer effects). Returns
/// `None` if the NID is not owned by this module; the caller
/// chains to the next module.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    match nid {
        NID_SYS_INITIALIZE_TLS => {
            initialize_tls(&mut adapter(runtime, source, nid), args);
        }
        NID_SYS_PROCESS_EXIT => {
            process_exit(&mut adapter(runtime, source, nid), args);
        }
        NID_SYS_MALLOC => {
            malloc(&mut adapter(runtime, source, nid), args);
        }
        NID_SYS_FREE | NID_SYS_HEAP_DELETE_HEAP | NID_SYS_HEAP_FREE => {
            // No-op. The HLE bump allocator in `hle::context`
            // cannot release individual allocations, so free,
            // delete-heap, and heap-free collapse to CELL_OK with
            // the allocation leaked. Same compromise RPCS3's HLE
            // path makes for the same reason.
            //
            // TODO: Replace with a real free-list allocator when a
            // scenario starts exercising the HLE heap.
            //
            //   TRIGGER: watch [`crate::Runtime::hle_heap_watermark`]
            //   in run-game output. Today (04/19/26) every scenario we run
            //   reports "0 bytes used above base" -- no scenario allocates
            //   on the HLE heap at all. Post-boot game-loop code
            //   will hit it eventually (asset streaming, per-frame
            //   scene allocations). When any run shows nonzero
            //   usage, or when usage crosses ~50% of the configured
            //   arena size, switch from bump to free-list.
            //
            //   DESIGN (~80-120 lines):
            //     struct HleHeap {
            //         heap_ptr: u32,              // bump cursor
            //         live: BTreeMap<u32, u32>,   // addr -> size
            //         free: BTreeMap<u32, u32>,   // addr -> size
            //     }
            //   heap_alloc: first-fit over `free` (iterate, find
            //     smallest chunk >= size, split remainder back),
            //     fall back to bump. Insert into `live`.
            //   free_alloc(ptr): look up size in `live`, remove,
            //     insert into `free`, coalesce with immediate
            //     predecessor/successor ranges in `free`.
            //   Determinism: BTreeMap iteration is stable, so
            //   alloc sequences stay reproducible.
            //
            //   RISKS to re-evaluate at implementation time:
            //     - Address recycling changes observation shape.
            //       Cross-runner comparison currently tolerates
            //       bump addresses via delta-from-base
            //       normalization; recycled addresses may need
            //       additional comparison-side logic.
            //     - Guest UAF becomes observable. Reclaimed bytes
            //       will differ from the UAF'd pointer's
            //       expectation, producing divergence from RPCS3's
            //       leak-mode. This is strictly better oracle
            //       behavior (surfaces real bugs) but is a
            //       behavior change for the specific case of
            //       buggy guests.
            adapter(runtime, source, nid).set_return(0);
        }
        NID_SYS_MEMSET => {
            memset(&mut adapter(runtime, source, nid), args);
        }
        NID_SYS_LWMUTEX_CREATE => {
            lwmutex_create(&mut adapter(runtime, source, nid), args);
        }
        NID_SYS_HEAP_CREATE_HEAP => {
            heap_create_heap(&mut adapter(runtime, source, nid));
        }
        NID_SYS_HEAP_MALLOC => {
            heap_malloc(&mut adapter(runtime, source, nid), args);
        }
        NID_SYS_HEAP_MEMALIGN => {
            heap_memalign(&mut adapter(runtime, source, nid), args);
        }
        NID_SYS_PPU_THREAD_GET_ID => {
            // Look up the caller's guest-facing PPU thread id
            // from the LV2 host's thread table. By the time guest
            // code runs, the table must be seeded -- an unseeded
            // table means two distinct units both collapse to the
            // fallback 0x01000000 and break any guest code keyed
            // on thread-id (TLS-keyed maps, reader/writer
            // tracking). Debug builds assert; release keeps the
            // fallback so shipped boots do not panic on an
            // unexpected call ordering.
            let table_id = runtime.lv2_host().ppu_thread_id_for_unit(source);
            debug_assert!(
                table_id.is_some(),
                "sys_ppu_thread_get_id: LV2 thread table not seeded for unit {source:?}; \
                 fallback id 0x01000000 would collide with every other unit's call"
            );
            let id: u64 = table_id.map(|tid| tid.raw()).unwrap_or(0x0100_0000);
            let ptr = args[0] as u32;
            let mut ctx = adapter(runtime, source, nid);
            ctx.write_guest(ptr as u64, &id.to_be_bytes())
                .expect("sys_ppu_thread_get_id: write to caller out-ptr failed");
            ctx.set_return(0);
        }
        NID_SYS_TIME_GET_SYSTEM_TIME => {
            // Deterministic fixed time (1 second since boot in
            // microseconds). The oracle values replay determinism
            // over wall-clock accuracy: guest code that seeds RNG
            // from this value produces the same sequence across
            // runs, and guest code that computes time deltas will
            // always see zero elapsed. If a future scenario needs
            // monotonic advance, wire through `runtime.time()`
            // (GuestTicks) rather than a host clock -- host time
            // would break the determinism contract.
            adapter(runtime, source, nid).set_return(1_000_000);
        }
        NID_SYS_THREAD_CREATE_EX => {
            // PSL1GHT's sysThreadCreateEx maps directly onto
            // sys_ppu_thread_create. Arg layout matches: r3
            // through r8 carry id_ptr, opd_ptr, arg, priority,
            // stacksize, flags.
            //
            // ABI narrowing: id_ptr and entry_opd are 32-bit
            // guest pointers and priority is a 32-bit value, so
            // `as u32` is the canonical narrowing cast and not a
            // lossy truncation. arg/stacksize/flags stay u64 to
            // match the Lv2Request field types (see
            // `cellgov_lv2::Lv2Request::PpuThreadCreate`).
            runtime.dispatch_lv2_request(
                cellgov_lv2::Lv2Request::PpuThreadCreate {
                    id_ptr: args[0] as u32,
                    entry_opd: args[1] as u32,
                    arg: args[2],
                    priority: args[3] as u32,
                    stacksize: args[4],
                    flags: args[5],
                },
                source,
            );
        }
        NID_SYS_THREAD_EXIT => {
            runtime.dispatch_lv2_request(
                cellgov_lv2::Lv2Request::PpuThreadExit {
                    exit_value: args[0],
                },
                source,
            );
        }
        _ => return None,
    }
    Some(())
}

fn adapter(runtime: &mut Runtime, source: UnitId, nid: u32) -> RuntimeHleAdapter<'_> {
    RuntimeHleAdapter {
        memory: &mut runtime.memory,
        registry: &mut runtime.registry,
        heap_base: runtime.hle.heap_base,
        heap_ptr: &mut runtime.hle.heap_ptr,
        heap_watermark: &mut runtime.hle.heap_watermark,
        heap_warning_mask: &mut runtime.hle.heap_warning_mask,
        next_id: &mut runtime.hle.next_id,
        source,
        nid,
        mutated: false,
        handlers_without_mutation: &mut runtime.hle.handlers_without_mutation,
    }
}

pub(crate) fn initialize_tls(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let tls_seg_addr = args[2] as u32;
    let tls_seg_size = args[3] as u32;
    let tls_mem_size = args[4] as u32;

    // ELF PT_TLS invariant: p_filesz (tls_seg_size) <= p_memsz
    // (tls_mem_size). The args reach this handler from the ELF
    // loader, not the guest -- violating this invariant means
    // either the loader is buggy or the ELF is malformed. Assert
    // so test harnesses surface it; release falls through to the
    // prior "skip on out-of-range" behavior so shipped boots do
    // not panic on unusual-but-valid TLS layouts.
    debug_assert!(
        tls_seg_size <= tls_mem_size,
        "sys_initialize_tls: malformed TLS (p_filesz={tls_seg_size:#x} > \
         p_memsz={tls_mem_size:#x})"
    );

    let tls_base: u32 = 0x10400000;

    let src = tls_seg_addr as usize;
    // tls_base + 0x30 is constant + constant; no wrap possible.
    let dst = (tls_base + 0x30) as usize;
    let copy_len = tls_seg_size as usize;
    let src_end = src.saturating_add(copy_len);
    let dst_end = dst.saturating_add(copy_len);
    let mem_len = ctx.guest_memory_len();
    debug_assert!(
        src_end <= mem_len && dst_end <= mem_len,
        "sys_initialize_tls: TLS segment [{src:#x}..{src_end:#x}] or slot \
         [{dst:#x}..{dst_end:#x}] out of guest memory (len={mem_len:#x}); \
         unplaceable TLS segment (normally from the ELF loader, but also \
         reachable from fuzz/synthetic test args)"
    );
    let init_data: Vec<u8> = if src_end <= mem_len && dst_end <= mem_len {
        ctx.guest_memory()[src..src_end].to_vec()
    } else {
        Vec::new()
    };
    if !init_data.is_empty() {
        ctx.write_guest(dst as u64, &init_data)
            .expect("sys_initialize_tls: TLS init-data copy failed");
    }

    let bss_start = dst_end;
    // bss_len = tls_mem_size - tls_seg_size (after cancellation of
    // the 0x30 header offset both sides). Uses wrapping_sub;
    // malformed ELFs with p_filesz > p_memsz wrap to a huge value
    // here and get rejected by the bounds check below. The
    // debug_assert at the top of the function catches that case
    // in tests.
    let bss_len = tls_mem_size.wrapping_sub(tls_seg_size) as usize;
    let bss_end = bss_start.saturating_add(bss_len);
    if bss_len > 0 && bss_end <= mem_len {
        let zeros = vec![0u8; bss_len];
        ctx.write_guest(bss_start as u64, &zeros)
            .expect("sys_initialize_tls: TLS bss zeroing failed");
    }

    // tls_base + 0x30 + 0x7000 = 0x10407030; all constants, no wrap.
    let r13_val = (tls_base + 0x30 + 0x7000) as u64;
    ctx.set_register(13, r13_val);
    ctx.set_return(0);
}

pub(crate) fn malloc(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let size = args[1] as u32;
    let ptr = ctx
        .heap_alloc(size, 16)
        .expect("_sys_malloc: HLE heap exhausted");
    ctx.set_return(ptr as u64);
}

pub(crate) fn memset(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let ptr = args[1] as u32;
    let val = args[2] as u8;
    let size = args[3] as u32;
    // Zero-length memset is a libc-legal no-op: return the
    // pointer, touch nothing.
    if size == 0 {
        ctx.set_return(args[1]);
        return;
    }
    // Real libc memset does not validate; it writes and faults if
    // the page is bad. The oracle cannot produce PPU page faults
    // from a guest-supplied bad pointer, so route through
    // write_guest and map any error (invalid range, unmapped
    // region) to CELL_EFAULT. Prior behavior silently skipped the
    // write and returned the pointer as though it succeeded; the
    // guest then read uninitialized memory and diverged
    // non-deterministically.
    //
    // Known divergence from real libc semantics: real memset never
    // returns an error -- it returns the destination pointer
    // unconditionally and faults internally on a bad page. A
    // caller doing `if (memset(p, 0, n) == NULL)` on real hardware
    // would never take the error branch because memset never
    // returns NULL (or CELL_EFAULT, which is also non-zero and
    // pointer-shaped). Well-behaved libc callers do not check
    // memset's return value, so this divergence is acceptable for
    // the oracle's purposes; the tradeoff is a visible diff in
    // behavior for buggy guests that do check it, in exchange for
    // the oracle not silently reporting success on a skipped
    // write.
    let data = vec![val; size as usize];
    match ctx.write_guest(ptr as u64, &data) {
        Ok(()) => ctx.set_return(args[1]),
        Err(_) => ctx.set_return(CELL_EFAULT),
    }
}

pub(crate) fn lwmutex_create(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let mutex_ptr = args[1] as u32;
    let attr_ptr = args[2] as u32;

    // RPCS3's `sys_lwmutex_create` accesses the attr struct via
    // `vm::ptr<sys_lwmutex_attribute_t>`, which auto-faults on a
    // bad pointer. Our HLE path does the analogous check
    // explicitly: bad attr_ptr -> CELL_EFAULT. The previous
    // implementation silently substituted (PRIORITY, NOT_RECURSIVE)
    // defaults and reported CELL_OK, which gave guests a mutex
    // with attributes they never requested and broke any code
    // that depended on PRIO or recursive semantics.
    let mem = ctx.guest_memory();
    let attr_offset = attr_ptr as usize;
    let Some(attr_end) = attr_offset.checked_add(8) else {
        ctx.set_return(CELL_EFAULT);
        return;
    };
    if attr_end > mem.len() {
        ctx.set_return(CELL_EFAULT);
        return;
    }
    let protocol = u32::from_be_bytes([
        mem[attr_offset],
        mem[attr_offset + 1],
        mem[attr_offset + 2],
        mem[attr_offset + 3],
    ]);
    let recursive = u32::from_be_bytes([
        mem[attr_offset + 4],
        mem[attr_offset + 5],
        mem[attr_offset + 6],
        mem[attr_offset + 7],
    ]);

    let sleep_queue = ctx
        .alloc_id()
        .expect("sys_lwmutex_create: HLE id counter exhausted");

    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
    buf[8..12].copy_from_slice(&(recursive | protocol).to_be_bytes());
    buf[16..20].copy_from_slice(&sleep_queue.to_be_bytes());

    // Write the mutex struct into the guest-supplied slot. A bad
    // mutex_ptr yields CELL_EFAULT (same as RPCS3's vm::ptr
    // auto-validation). Prior behavior silently skipped the
    // write, leaking the sleep_queue ID and handing the guest
    // back CELL_OK on an uninitialized mutex struct.
    match ctx.write_guest(mutex_ptr as u64, &buf) {
        Ok(()) => ctx.set_return(0),
        Err(_) => ctx.set_return(CELL_EFAULT),
    }
}

pub(crate) fn heap_create_heap(ctx: &mut dyn HleContext) {
    let id = ctx
        .alloc_id()
        .expect("sys_heap_create_heap: HLE id counter exhausted");
    ctx.set_return(id as u64);
}

pub(crate) fn heap_malloc(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let size = args[2] as u32;
    let ptr = ctx
        .heap_alloc(size, 16)
        .expect("sys_heap_malloc: HLE heap exhausted");
    ctx.set_return(ptr as u64);
}

pub(crate) fn heap_memalign(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let align = (args[2] as u32).max(16);
    let size = args[3] as u32;
    let ptr = ctx
        .heap_alloc(size, align)
        .expect("sys_heap_memalign: HLE heap exhausted");
    ctx.set_return(ptr as u64);
}

pub(crate) fn process_exit(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    // Forward the guest-supplied exit code. The unit transitions
    // to Finished so it never resumes, but set_return records the
    // value in the registry's syscall-return slot so a harness
    // comparing process-exit codes can distinguish a clean exit
    // (0) from a crash-path exit (0x80). Mirrors the
    // NID_SYS_THREAD_EXIT path which already forwards args[0] as
    // exit_value to the LV2 dispatcher.
    //
    // Design note: set_return now carries two orthogonal
    // meanings -- "value r3 will hold on the unit's next resume"
    // (every other call site) and "value to record for
    // post-mortem inspection on a unit that will never resume"
    // (this call site alone, for now). The registry does not
    // distinguish them, so if a future change clears the return
    // slot on Finished units this exit code vanishes. If this
    // coupling grows a second caller, promote to an explicit
    // HleContext::set_exit_code method that the registry handles
    // as a distinct slot.
    ctx.set_unit_finished();
    ctx.set_return(args[0]);
}

#[cfg(test)]
mod canary_tests {
    use super::{dispatch, OWNED_NIDS};
    use crate::runtime::Runtime;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_lv2::PpuThreadAttrs;
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    /// Build a runtime wired up just enough for any single sys-module
    /// NID dispatch to reach its handler: a registered primary unit,
    /// a seeded LV2 PPU thread table (for PPU_THREAD_GET_ID), a heap
    /// base (for the malloc family), and a stub PPU factory (for
    /// THREAD_CREATE_EX). The canary is indifferent to what the
    /// handlers *do*; it only checks that dispatch claims the NID.
    fn canary_runtime() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10000);
        rt.set_ppu_factory(|id, _init| Box::new(FakeIsaUnit::new(id, vec![FakeOp::End])));
        rt.lv2_host_mut().seed_primary_ppu_thread(
            unit_id,
            PpuThreadAttrs {
                entry: 0x1000,
                arg: 0,
                stack_base: 0xD000_0000,
                stack_size: 0x10000,
                priority: 1000,
                tls_base: 0,
            },
        );
        (rt, unit_id)
    }

    /// Drift canary for [`OWNED_NIDS`] vs the [`dispatch`] match arms.
    ///
    /// For every NID named in the list, invoke `dispatch` and assert
    /// it claimed the NID (either returned `Some(())` or reached the
    /// handler and panicked on synthetic-zero args). A match arm
    /// removed without also removing its NID from the list flips
    /// the corresponding call to `None` and this test fires with
    /// the specific NID that drifted. Keeps the list honest without
    /// a macro refactor.
    ///
    /// ## Why `catch_unwind` is load-bearing
    ///
    /// The contract this canary pins is "dispatch routed the NID to
    /// a handler," not "the handler succeeded." Handlers dispatched
    /// with all-zero args may legitimately trip their own
    /// `debug_assert!` invariants (e.g., `initialize_tls` asserts its
    /// ELF-supplied TLS segment fits in guest memory; our 1 MiB
    /// canary fixture is smaller than the hardcoded TLS base).
    /// `catch_unwind` treats those panics as evidence the handler
    /// was reached, which is precisely what the canary needs. Do
    /// not "fix" the canary by seeding every possible handler
    /// precondition -- that would be a different test (handler
    /// correctness) and would mask genuine drift by hiding it
    /// behind a compounding fixture. If a handler panics on
    /// routing-only args and you want a separate test of its
    /// behavior with real args, write a dedicated test.
    #[test]
    fn owned_nids_all_claimed_by_dispatch() {
        for &nid in OWNED_NIDS {
            let (mut rt, unit_id) = canary_runtime();
            let args: [u64; 9] = [0; 9];
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                dispatch(&mut rt, unit_id, nid, &args)
            }));
            match result {
                Ok(Some(())) => {}
                Ok(None) => panic!(
                    "sys::dispatch returned None for NID {nid:#010x} listed in OWNED_NIDS \
                     -- the match arm was likely removed without trimming the list"
                ),
                Err(_) => {
                    // Handler panicked -> NID was claimed and routed
                    // into a handler body. Canary's "is this NID
                    // dispatched?" question is answered yes.
                }
            }
        }
    }

    /// Negative companion to the coverage canary: NIDs that do not
    /// belong to this module must not be claimed by it. Sampling a
    /// handful of gcm NIDs and a synthetic never-registered NID
    /// catches the other drift direction (a NID added to
    /// [`OWNED_NIDS`] that does not appear in any match arm would
    /// make the positive canary pass, but this negative check pins
    /// the policy that sys really does return `None` outside its
    /// owned set).
    #[test]
    fn unowned_nids_are_rejected_by_dispatch() {
        let probes: &[u32] = &[
            crate::hle::cell_gcm_sys::NID_CELLGCM_INIT_BODY,
            crate::hle::cell_gcm_sys::NID_CELLGCM_GET_CONFIGURATION,
            0xDEAD_BEEF,
        ];
        for &nid in probes {
            let (mut rt, unit_id) = canary_runtime();
            let args: [u64; 9] = [0; 9];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result, None,
                "sys::dispatch claimed NID {nid:#010x} that is not in its OWNED_NIDS"
            );
        }
    }
}
