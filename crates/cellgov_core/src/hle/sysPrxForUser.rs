//! sysPrxForUser HLE implementations.
//!
//! Kernel-side syscalls (`sc` trap handlers) live in `cellgov_lv2`.
//!
//! ## Failure policy
//!
//! - Invariants that only our loader/runtime can violate (malformed
//!   TLS, unseeded LV2 thread table) use `debug_assert!`; release
//!   keeps the historical fallback.
//! - Guest-supplied bad pointers return `CELL_EFAULT` (matching
//!   Sony's trap-on-bad-pointer via RPCS3's `vm::ptr`).
//! - `.expect(...)` is reserved for oracle-state corruption (heap
//!   exhaustion, ID counter exhaustion).

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors::CELL_EFAULT;
use cellgov_ps3_abi::nid::sys_prx_for_user as sys_nid;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

/// Every NID this module claims; sourced from
/// [`cellgov_ps3_abi::nid::sys_prx_for_user::OWNED`].
#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = sys_nid::OWNED;

/// Dispatch entry point; returns `None` if the NID is not owned here.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    match nid {
        sys_nid::INITIALIZE_TLS => {
            initialize_tls(&mut adapter(runtime, source, nid), args);
        }
        sys_nid::PROCESS_EXIT => {
            process_exit(&mut adapter(runtime, source, nid), args);
        }
        sys_nid::MALLOC => {
            malloc(&mut adapter(runtime, source, nid), args);
        }
        sys_nid::FREE | sys_nid::HEAP_DELETE_HEAP | sys_nid::HEAP_FREE => {
            // No-op: the HLE bump allocator in `hle::context` cannot
            // release individual allocations, so free / delete-heap
            // / heap-free collapse to CELL_OK with the allocation
            // leaked (RPCS3's HLE path makes the same compromise).
            //
            // TODO: switch to a real free-list allocator when
            // `hle_heap_watermark` shows non-trivial usage.
            adapter(runtime, source, nid).set_return(0);
        }
        sys_nid::MEMSET => {
            memset(&mut adapter(runtime, source, nid), args);
        }
        sys_nid::LWMUTEX_CREATE => {
            lwmutex_create(&mut adapter(runtime, source, nid), args);
        }
        sys_nid::HEAP_CREATE_HEAP => {
            heap_create_heap(&mut adapter(runtime, source, nid));
        }
        sys_nid::HEAP_MALLOC => {
            heap_malloc(&mut adapter(runtime, source, nid), args);
        }
        sys_nid::HEAP_MEMALIGN => {
            heap_memalign(&mut adapter(runtime, source, nid), args);
        }
        sys_nid::PPU_THREAD_GET_ID => {
            // Look up the guest-facing thread id via the LV2 table.
            // An unseeded table collapses every unit to the fallback
            // 0x01000000, which breaks thread-id-keyed state.
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
        sys_nid::TIME_GET_SYSTEM_TIME => {
            // Fixed 1 second in microseconds. Determinism beats
            // wall-clock accuracy; a monotonic source would have to
            // come from `runtime.time()` (GuestTicks), never a host
            // clock.
            adapter(runtime, source, nid).set_return(1_000_000);
        }
        sys_nid::PPU_THREAD_CREATE => {
            // sysPrxForUser SDK wrapper, NID 0x24a1ea07. Signature
            // (per RPCS3's sysPrxForUser.cpp):
            //   sys_ppu_thread_create(
            //       thread_id*,    // r3 = args[1]: out-pointer
            //       u32 entry,     // r4 = args[2]: entry OPD address
            //                      //   (DIRECT, not a struct pointer)
            //       u64 arg,       // r5 = args[3]
            //       s32 prio,      // r6 = args[4]
            //       u32 stacksize, // r7 = args[5]
            //       u64 flags,     // r8 = args[6]
            //       char* name,    // r9 = args[7]
            //   )
            //
            // The SDK wrapper allocates the LV2-side param struct
            // internally and calls into _sys_ppu_thread_create (LV2
            // syscall 52) where r4 is the struct pointer. The HLE
            // entry takes the entry OPD raw -- no param-struct
            // dereference is needed here.
            let entry_opd = args[2] as u32;
            if entry_opd == 0 {
                adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
                return Some(());
            }
            runtime.dispatch_lv2_request(
                cellgov_lv2::Lv2Request::PpuThreadCreate {
                    id_ptr: args[1] as u32,
                    entry_opd,
                    arg: args[3],
                    priority: args[4] as u32,
                    stacksize: args[5],
                    flags: args[6],
                },
                source,
            );
        }
        sys_nid::PPU_THREAD_EXIT => {
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

    // ELF PT_TLS invariant: p_filesz <= p_memsz. A violation means
    // a buggy loader or malformed ELF.
    debug_assert!(
        tls_seg_size <= tls_mem_size,
        "sys_initialize_tls: malformed TLS (p_filesz={tls_seg_size:#x} > \
         p_memsz={tls_mem_size:#x})"
    );

    let tls_base: u32 = 0x10400000;

    let src = tls_seg_addr as usize;
    let dst = (tls_base + 0x30) as usize;
    let copy_len = tls_seg_size as usize;
    let src_end = src.saturating_add(copy_len);
    let dst_end = dst.saturating_add(copy_len);
    let mem_len = ctx.guest_memory_len();
    debug_assert!(
        src_end <= mem_len && dst_end <= mem_len,
        "sys_initialize_tls: TLS segment [{src:#x}..{src_end:#x}] or slot \
         [{dst:#x}..{dst_end:#x}] out of guest memory (len={mem_len:#x})"
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
    // Malformed p_filesz > p_memsz wraps here to a huge value and
    // is rejected by the bounds check below.
    let bss_len = tls_mem_size.wrapping_sub(tls_seg_size) as usize;
    let bss_end = bss_start.saturating_add(bss_len);
    if bss_len > 0 && bss_end <= mem_len {
        let zeros = vec![0u8; bss_len];
        ctx.write_guest(bss_start as u64, &zeros)
            .expect("sys_initialize_tls: TLS bss zeroing failed");
    }

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
    if size == 0 {
        ctx.set_return(args[1]);
        return;
    }
    // Real libc memset never returns an error; it faults on a bad
    // page, which the oracle cannot produce. Map a failed write to
    // CELL_EFAULT instead of silently skipping it. Guests that
    // check memset's return value see a visible diff here.
    let data = vec![val; size as usize];
    match ctx.write_guest(ptr as u64, &data) {
        Ok(()) => ctx.set_return(args[1]),
        Err(_) => ctx.set_return(CELL_EFAULT.into()),
    }
}

pub(crate) fn lwmutex_create(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let mutex_ptr = args[1] as u32;
    let attr_ptr = args[2] as u32;

    // Sony's sys_lwmutex_create traps on a bad attr_ptr; match that
    // with an explicit CELL_EFAULT rather than substituting
    // (PRIORITY, NOT_RECURSIVE) defaults.
    let mem = ctx.guest_memory();
    let attr_offset = attr_ptr as usize;
    let Some(attr_end) = attr_offset.checked_add(8) else {
        ctx.set_return(CELL_EFAULT.into());
        return;
    };
    if attr_end > mem.len() {
        ctx.set_return(CELL_EFAULT.into());
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

    // Bad mutex_ptr -> CELL_EFAULT, matching on-device trap-on-write.
    match ctx.write_guest(mutex_ptr as u64, &buf) {
        Ok(()) => ctx.set_return(0),
        Err(_) => ctx.set_return(CELL_EFAULT.into()),
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
    // Finished units never resume; set_return here repurposes the
    // registry's syscall-return slot as a post-mortem exit-code
    // carrier. A registry change that clears the return slot on
    // Finished units would silently drop the exit code; if a
    // second caller needs this, promote to a dedicated
    // `HleContext::set_exit_code` method.
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

    /// Runtime wired up just enough for any sys-module NID to reach
    /// its handler (registered unit, seeded LV2 thread table, heap
    /// base, stub PPU factory).
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

    /// Drift canary: every NID in [`OWNED_NIDS`] must reach a
    /// handler. A handler panic on synthetic-zero args counts as
    /// "routed" -- `catch_unwind` captures it as evidence of
    /// dispatch reaching the body.
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
                    // Handler panicked => NID reached the body.
                }
            }
        }
    }

    /// Negative companion: gcm-owned and synthetic NIDs must return
    /// `None`.
    #[test]
    fn unowned_nids_are_rejected_by_dispatch() {
        let probes: &[u32] = &[
            cellgov_ps3_abi::nid::cell_gcm_sys::INIT_BODY,
            cellgov_ps3_abi::nid::cell_gcm_sys::GET_CONFIGURATION,
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
