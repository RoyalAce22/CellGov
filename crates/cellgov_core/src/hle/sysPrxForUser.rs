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
use cellgov_lv2::errno::CELL_EFAULT;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

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

/// Every NID this module claims. Consumed by the disjointness and
/// dispatch-coverage canaries in `crate::hle::tests` and
/// `canary_tests`.
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

/// Dispatch entry point; returns `None` if the NID is not owned here.
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
            // No-op: the HLE bump allocator in `hle::context` cannot
            // release individual allocations, so free / delete-heap
            // / heap-free collapse to CELL_OK with the allocation
            // leaked (RPCS3's HLE path makes the same compromise).
            //
            // TODO: switch to a real free-list allocator when
            // `hle_heap_watermark` shows non-trivial usage.
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
        NID_SYS_TIME_GET_SYSTEM_TIME => {
            // Fixed 1 second in microseconds. Determinism beats
            // wall-clock accuracy; a monotonic source would have to
            // come from `runtime.time()` (GuestTicks), never a host
            // clock.
            adapter(runtime, source, nid).set_return(1_000_000);
        }
        NID_SYS_THREAD_CREATE_EX => {
            // r4 (args[1]) is a POINTER to a param struct, not the
            // entry OPD: the struct carries the 8-byte BE entry-OPD
            // at offset 0 and the TLS address at offset 8. Reading
            // args[1] as an OPD directly would spawn a thread whose
            // PC is whatever 8 bytes live at the top of the struct.
            //
            // Register layout: r3 thread_id out, r4 param ptr, r5
            // arg, r6 reserved (0), r7 prio, r8 stacksize, r9
            // flags, r10 threadname. The reserved slot at r6 shifts
            // prio / stacksize / flags one register up.
            let param_ptr = args[1] as u32;
            let param_start = param_ptr as usize;
            let entry_opd_read: Option<u32> = {
                let ctx = adapter(runtime, source, nid);
                let mem = ctx.guest_memory();
                mem.get(param_start..param_start + 8)
                    .map(|slice| u64::from_be_bytes(slice.try_into().unwrap()) as u32)
            };
            let entry_opd = match entry_opd_read {
                Some(opd) if opd != 0 => opd,
                _ => {
                    adapter(runtime, source, nid)
                        .set_return(cellgov_lv2::errno::CELL_EFAULT.into());
                    return Some(());
                }
            };
            runtime.dispatch_lv2_request(
                cellgov_lv2::Lv2Request::PpuThreadCreate {
                    id_ptr: args[0] as u32,
                    entry_opd,
                    arg: args[2],
                    priority: args[4] as u32,
                    stacksize: args[5],
                    flags: args[6],
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
