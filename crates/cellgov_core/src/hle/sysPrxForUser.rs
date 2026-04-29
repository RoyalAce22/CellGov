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
        sys_nid::PROCESS_IS_STACK => {
            // Real LV2: returns 1 iff `addr` is in a PPU thread
            // stack region. CellGov configures the primary stack at
            // 0xD0000000+0x10000 and child stacks at 0xD0010000 +
            // 0xF00000; the lower bound is widened to 0xCFF00000 to
            // accommodate PSL1GHT primary-thread startup that
            // consumes more than 64 KiB before main() runs and
            // leaves locals just below the configured stack base.
            //
            // HLE call convention: args[0] is the trampoline-side
            // syscall-id slot; args[1] holds the first guest C-level
            // argument (the pointer to test).
            let addr = args[1] as u32;
            const STACK_RANGE_LO: u32 = 0xCFF0_0000;
            const STACK_RANGE_HI: u32 = 0xD0F1_0000;
            let in_stack = (STACK_RANGE_LO..STACK_RANGE_HI).contains(&addr);
            adapter(runtime, source, nid).set_return(if in_stack { 1 } else { 0 });
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
            lwmutex_create(runtime, source, nid, args);
        }
        sys_nid::LWMUTEX_LOCK => {
            lwmutex_lock_hle(runtime, source, nid, args);
        }
        sys_nid::LWMUTEX_UNLOCK => {
            lwmutex_unlock_hle(runtime, source, nid, args);
        }
        sys_nid::LWMUTEX_TRYLOCK => {
            lwmutex_trylock_hle(runtime, source, nid, args);
        }
        sys_nid::LWMUTEX_DESTROY => {
            lwmutex_destroy_hle(runtime, source, nid, args);
        }
        sys_nid::LWCOND_CREATE => {
            lwcond_create(runtime, source, nid, args);
        }
        sys_nid::LWCOND_DESTROY => {
            // Stub: decrement count via the LV2 host. The lwcond
            // queue/signal surface is not yet modeled; fully wiring
            // it is its own slice.
            runtime.lv2_host_mut().lwcond_count_dec();
            adapter(runtime, source, nid).set_return(0);
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
            // Microseconds since boot, derived from the deterministic
            // guest clock (1 tick = 1 ns, so us = ticks / 1000). The
            // timer syscalls advance `runtime.time` so a thread that
            // measures `system_time()` before and after a sleep sees
            // the expected delta.
            let us = runtime.time().raw() / 1_000;
            adapter(runtime, source, nid).set_return(us);
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

/// `sys_lwmutex_create` HLE shim.
///
/// Allocates the lwmutex's `sleep_queue` id from the LV2 lwmutex
/// table so subsequent `sys_lwmutex_{lock,unlock,trylock,destroy}`
/// routed through [`lwmutex_route`] resolve through the same
/// blocking surface.
pub(crate) fn lwmutex_create(runtime: &mut Runtime, source: UnitId, nid: u32, args: &[u64; 9]) {
    let mutex_ptr = args[1] as u32;
    let attr_ptr = args[2] as u32;

    // Sony's sys_lwmutex_create traps on a bad attr_ptr; match that
    // with an explicit CELL_EFAULT rather than substituting
    // (PRIORITY, NOT_RECURSIVE) defaults. The read is region-aware
    // because guests pass stack-allocated attrs (PSL1GHT puts the
    // primary thread stack at 0xD0000000+); a linear `as_bytes()`
    // probe would always fail those addresses.
    let (protocol, recursive) = {
        use cellgov_mem::ByteRange;
        let Some(range) = ByteRange::new(cellgov_mem::GuestAddr::new(attr_ptr as u64), 8) else {
            adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
            return;
        };
        let Some(bytes) = runtime.memory.read(range) else {
            adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
            return;
        };
        let protocol = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let recursive = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        (protocol, recursive)
    };

    let Some(sleep_queue) = runtime.lv2_host_mut().lwmutexes_mut().create() else {
        adapter(runtime, source, nid).set_return(cellgov_ps3_abi::cell_errors::CELL_ENOMEM.into());
        return;
    };

    // Initialise the full struct: owner = FREE, waiter = 0,
    // attribute = recursive | protocol, recursive_count = 0,
    // sleep_queue = kernel id, pad = 0. Leaving waiter or
    // recursive_count uninitialised would let stack garbage drive
    // the user-space fast path and force spurious kernel calls.
    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
    // buf[4..8] = waiter = 0 (already zero).
    buf[8..12].copy_from_slice(&(recursive | protocol).to_be_bytes());
    // buf[12..16] = recursive_count = 0 (already zero).
    buf[16..20].copy_from_slice(&sleep_queue.to_be_bytes());
    // buf[20..24] = pad = 0 (already zero).

    let mut ctx = adapter(runtime, source, nid);
    match ctx.write_guest(mutex_ptr as u64, &buf) {
        Ok(()) => ctx.set_return(0),
        Err(_) => ctx.set_return(CELL_EFAULT.into()),
    }
}

/// `sys_lwcond_create` HLE shim.
///
/// Stub: tracks live-object count for `sys_process_get_number_of_object`
/// and writes a placeholder kernel id back to the user lwcond
/// struct. Wait/signal semantics are not modeled.
pub(crate) fn lwcond_create(runtime: &mut Runtime, source: UnitId, nid: u32, args: &[u64; 9]) {
    let lwcond_ptr = args[1] as u32;
    let _lwmutex_ptr = args[2] as u32; // paired lwmutex; unused for the count-only stub
    let _attr_ptr = args[3] as u32;

    runtime.lv2_host_mut().lwcond_count_inc();

    // Place a non-zero id at offset 0 of the lwcond struct so any
    // caller probing the handle sees a non-zero value. The exact
    // layout of `sys_lwcond_t` is opaque here; this is purely a
    // visibility marker for the count-only stub. A full lwcond
    // implementation would populate the queue id and lwmutex
    // pointer fields.
    let id = 0xFFFFFFFFu32;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&id.to_be_bytes());
    let mut ctx = adapter(runtime, source, nid);
    let _ = ctx.write_guest(lwcond_ptr as u64, &buf);
    ctx.set_return(0);
}

/// Read the embedded `sleep_queue` id at offset 0x10 of an
/// `sys_lwmutex_t` and dispatch the supplied `Lv2Request` through
/// the LV2 lwmutex surface. Currently unused: the lock / unlock /
/// trylock / destroy paths each do their own user-space CAS and
/// dispatch directly. Kept for the day a non-recursive primitive
/// (semaphore?) wants the same routing shape.
#[allow(dead_code)]
fn lwmutex_route<F>(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
    make_request: F,
) where
    F: FnOnce(u32, u64) -> cellgov_lv2::Lv2Request,
{
    let mutex_ptr = args[1] as u32;
    let timeout = args[2];

    // Region-aware: lwmutex structs are typically stack-allocated by
    // guests, and the primary stack at 0xD0000000+ is outside the
    // linear user-memory region.
    use cellgov_mem::ByteRange;
    let Some(range) = ByteRange::new(
        cellgov_mem::GuestAddr::new((mutex_ptr as u64).saturating_add(0x10)),
        4,
    ) else {
        adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
        return;
    };
    let Some(bytes) = runtime.memory.read(range) else {
        adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
        return;
    };
    let id = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);

    runtime.dispatch_lv2_request(make_request(id, timeout), source);
}

// PSL1GHT `sys_lwmutex_t` layout:
//   +0  u32 owner            (lock_var.owner; 0xFFFF_FFFF == free)
//   +4  u32 waiter            (number of threads in kernel sleep queue)
//   +8  u32 attribute         (recursive | protocol)
//   +12 u32 recursive_count
//   +16 u32 sleep_queue       (kernel-side id)
//   +20 u32 pad
const LWMUTEX_OFF_OWNER: u64 = 0;
const LWMUTEX_OFF_WAITER: u64 = 4;
const LWMUTEX_OFF_RECURSIVE_COUNT: u64 = 12;
const LWMUTEX_FREE_OWNER: u32 = 0xFFFF_FFFF;
const SYS_SYNC_RECURSIVE: u32 = 0x10;

/// Snapshot of a guest `sys_lwmutex_t`'s scalar fields.
struct LwMutexFields {
    owner: u32,
    waiter: u32,
    attribute: u32,
    recursive_count: u32,
    sleep_queue: u32,
}

/// Region-aware read of all scalar fields, or `None` on a bad
/// `mutex_ptr` (matches Sony's trap-on-bad-pointer).
fn read_lwmutex_fields(runtime: &Runtime, mutex_ptr: u32) -> Option<LwMutexFields> {
    use cellgov_mem::ByteRange;
    let range = ByteRange::new(cellgov_mem::GuestAddr::new(mutex_ptr as u64), 24)?;
    let bytes = runtime.memory.read(range)?;
    let read_u32 = |off: usize| {
        u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
    };
    Some(LwMutexFields {
        owner: read_u32(0),
        waiter: read_u32(4),
        attribute: read_u32(8),
        recursive_count: read_u32(12),
        sleep_queue: read_u32(16),
    })
}

/// Region-aware single-field write; on failure the wrapper reports
/// `CELL_EFAULT` upstream.
fn write_lwmutex_u32(ctx: &mut dyn HleContext, mutex_ptr: u32, off: u64, value: u32) -> bool {
    ctx.write_guest((mutex_ptr as u64) + off, &value.to_be_bytes())
        .is_ok()
}

/// Caller's PSL1GHT thread id encoded into the user-space owner
/// field. The HLE keeps the LV2 thread-id as the canonical caller
/// identity; the user-space owner field stores its raw u32 form so
/// future kernel calls can detect "this is the same thread".
fn caller_owner_id(runtime: &Runtime, source: UnitId) -> u32 {
    runtime
        .lv2_host()
        .ppu_thread_id_for_unit(source)
        .map(|tid| tid.raw() as u32)
        .unwrap_or(0)
}

/// `sys_lwmutex_lock` HLE wrapper.
///
/// Implements the user-space fast-path so the kernel only sees
/// actual contention. RPCS3's `sys_lwmutex_lock` (PRX-side, not the
/// `_sys_*` syscall) does the same staging.
pub(crate) fn lwmutex_lock_hle(runtime: &mut Runtime, source: UnitId, nid: u32, args: &[u64; 9]) {
    let mutex_ptr = args[1] as u32;
    let timeout = args[2];
    let Some(fields) = read_lwmutex_fields(runtime, mutex_ptr) else {
        adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
        return;
    };
    let me = caller_owner_id(runtime, source);
    let owner_alive = runtime
        .lv2_host()
        .ppu_threads()
        .is_owner_alive(fields.owner);
    if fields.owner == LWMUTEX_FREE_OWNER || !owner_alive {
        // Uncontended acquire (or stale owner from a thread that
        // already exited without unlocking -- treated as free so the
        // mutex is not orphaned forever).
        let was_stale = fields.owner != LWMUTEX_FREE_OWNER && !owner_alive;
        let me_tid = runtime.lv2_host().ppu_thread_id_for_unit(source);
        let mut ctx = adapter(runtime, source, nid);
        if !write_lwmutex_u32(&mut ctx, mutex_ptr, LWMUTEX_OFF_OWNER, me)
            || !write_lwmutex_u32(&mut ctx, mutex_ptr, LWMUTEX_OFF_RECURSIVE_COUNT, 1)
        {
            ctx.set_return(CELL_EFAULT.into());
            return;
        }
        ctx.set_return(0);
        drop(ctx);
        if was_stale {
            runtime
                .lv2_host_mut()
                .lwmutex_holds_clear(cellgov_lv2::PpuThreadId::new(fields.owner as u64));
        }
        if let Some(tid) = me_tid {
            runtime.lv2_host_mut().lwmutex_holds_inc(tid);
        }
        return;
    }
    if fields.owner == me {
        // Recursive: we already own the lwmutex. PSL1GHT's
        // user-space wrapper is responsible for the
        // recursive_count bookkeeping; the kernel-side hook just
        // returns OK so the wrapper can proceed.
        if (fields.attribute & SYS_SYNC_RECURSIVE) != 0 {
            adapter(runtime, source, nid).set_return(0);
            return;
        }
        adapter(runtime, source, nid).set_return(cellgov_ps3_abi::cell_errors::CELL_EDEADLK.into());
        return;
    }
    // Contention: bump the user-space waiter so a concurrent
    // unlocker invokes the kernel, then block in the kernel sleep
    // queue. On wake, the runtime fills in owner / waiter /
    // recursive_count via `PendingResponse::LwMutexWake`.
    {
        let mut ctx = adapter(runtime, source, nid);
        if !write_lwmutex_u32(
            &mut ctx,
            mutex_ptr,
            LWMUTEX_OFF_WAITER,
            fields.waiter.saturating_add(1),
        ) {
            ctx.set_return(CELL_EFAULT.into());
            return;
        }
    }
    runtime.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::LwMutexLock {
            id: fields.sleep_queue,
            mutex_ptr,
            timeout,
        },
        source,
    );
    // The dispatch parked the unit if it had to block; the
    // post-wake bookkeeping (decrement waiter, write owner = me,
    // recursive_count = 1) is performed by the runtime when it
    // resolves `PendingResponse::LwMutexWake`.
}

/// `sys_lwmutex_unlock` HLE wrapper.
pub(crate) fn lwmutex_unlock_hle(runtime: &mut Runtime, source: UnitId, nid: u32, args: &[u64; 9]) {
    let mutex_ptr = args[1] as u32;
    let Some(fields) = read_lwmutex_fields(runtime, mutex_ptr) else {
        adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
        return;
    };
    let me = caller_owner_id(runtime, source);
    if fields.owner != me {
        adapter(runtime, source, nid).set_return(cellgov_ps3_abi::cell_errors::CELL_EPERM.into());
        return;
    }
    if fields.recursive_count > 1 {
        let mut ctx = adapter(runtime, source, nid);
        if !write_lwmutex_u32(
            &mut ctx,
            mutex_ptr,
            LWMUTEX_OFF_RECURSIVE_COUNT,
            fields.recursive_count - 1,
        ) {
            ctx.set_return(CELL_EFAULT.into());
            return;
        }
        ctx.set_return(0);
        return;
    }
    // Final release: clear user-space ownership, then wake one
    // kernel-side waiter if the user-space waiter counter says any
    // are parked.
    let me_tid = runtime.lv2_host().ppu_thread_id_for_unit(source);
    {
        let mut ctx = adapter(runtime, source, nid);
        if !write_lwmutex_u32(&mut ctx, mutex_ptr, LWMUTEX_OFF_OWNER, LWMUTEX_FREE_OWNER)
            || !write_lwmutex_u32(&mut ctx, mutex_ptr, LWMUTEX_OFF_RECURSIVE_COUNT, 0)
        {
            ctx.set_return(CELL_EFAULT.into());
            return;
        }
    }
    // Decrement the per-thread hold count: this is a final unlock
    // (recursive_count went 1 -> 0), so the lwmutex is no longer in
    // the holder's critical-section set.
    if let Some(tid) = me_tid {
        runtime.lv2_host_mut().lwmutex_holds_dec(tid);
    }
    if fields.waiter > 0 {
        runtime.dispatch_lv2_request(
            cellgov_lv2::Lv2Request::LwMutexUnlock {
                id: fields.sleep_queue,
            },
            source,
        );
    } else {
        adapter(runtime, source, nid).set_return(0);
    }
}

/// `sys_lwmutex_destroy` HLE wrapper. Real LV2 rejects destroy of
/// a held lwmutex with `CELL_EBUSY`; the held check looks at the
/// user-space owner field, since the kernel side has no
/// ownership tracking.
pub(crate) fn lwmutex_destroy_hle(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) {
    let mutex_ptr = args[1] as u32;
    let Some(fields) = read_lwmutex_fields(runtime, mutex_ptr) else {
        adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
        return;
    };
    if fields.owner != LWMUTEX_FREE_OWNER {
        adapter(runtime, source, nid).set_return(cellgov_ps3_abi::cell_errors::CELL_EBUSY.into());
        return;
    }
    runtime.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::LwMutexDestroy {
            id: fields.sleep_queue,
        },
        source,
    );
}

/// `sys_lwmutex_trylock` HLE wrapper. Mirrors the lock fast-path
/// but never blocks: contention reports `CELL_EBUSY`.
pub(crate) fn lwmutex_trylock_hle(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) {
    let mutex_ptr = args[1] as u32;
    let Some(fields) = read_lwmutex_fields(runtime, mutex_ptr) else {
        adapter(runtime, source, nid).set_return(CELL_EFAULT.into());
        return;
    };
    let me = caller_owner_id(runtime, source);
    let owner_alive = runtime
        .lv2_host()
        .ppu_threads()
        .is_owner_alive(fields.owner);
    if fields.owner == LWMUTEX_FREE_OWNER || !owner_alive {
        let was_stale = fields.owner != LWMUTEX_FREE_OWNER && !owner_alive;
        let me_tid = runtime.lv2_host().ppu_thread_id_for_unit(source);
        let mut ctx = adapter(runtime, source, nid);
        if !write_lwmutex_u32(&mut ctx, mutex_ptr, LWMUTEX_OFF_OWNER, me)
            || !write_lwmutex_u32(&mut ctx, mutex_ptr, LWMUTEX_OFF_RECURSIVE_COUNT, 1)
        {
            ctx.set_return(CELL_EFAULT.into());
            return;
        }
        ctx.set_return(0);
        drop(ctx);
        if was_stale {
            runtime
                .lv2_host_mut()
                .lwmutex_holds_clear(cellgov_lv2::PpuThreadId::new(fields.owner as u64));
        }
        if let Some(tid) = me_tid {
            runtime.lv2_host_mut().lwmutex_holds_inc(tid);
        }
        return;
    }
    if fields.owner == me && (fields.attribute & SYS_SYNC_RECURSIVE) != 0 {
        let mut ctx = adapter(runtime, source, nid);
        if !write_lwmutex_u32(
            &mut ctx,
            mutex_ptr,
            LWMUTEX_OFF_RECURSIVE_COUNT,
            fields.recursive_count.saturating_add(1),
        ) {
            ctx.set_return(CELL_EFAULT.into());
            return;
        }
        ctx.set_return(0);
        return;
    }
    adapter(runtime, source, nid).set_return(cellgov_ps3_abi::cell_errors::CELL_EBUSY.into());
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

#[cfg(test)]
mod lwmutex_routing_tests {
    use super::*;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_lv2::PpuThreadAttrs;
    use cellgov_mem::GuestMemory;
    use cellgov_ps3_abi::nid::sys_prx_for_user as sys_nid;
    use cellgov_time::Budget;

    /// 1 MiB guest memory + seeded primary PPU thread is enough for
    /// lwmutex traffic from a single unit.
    fn lwmutex_runtime() -> (Runtime, UnitId, u32) {
        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10000);
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
        // mutex_ptr at 0x40000, attr_ptr at 0x40100. 24-byte mutex,
        // 8-byte attribute (zero protocol + non-recursive).
        let mutex_ptr: u32 = 0x40000;
        (rt, unit_id, mutex_ptr)
    }

    fn create_args(mutex_ptr: u32) -> [u64; 9] {
        let attr_ptr: u32 = 0x40100;
        [0, mutex_ptr as u64, attr_ptr as u64, 0, 0, 0, 0, 0, 0]
    }

    fn ptr_args(mutex_ptr: u32) -> [u64; 9] {
        [0, mutex_ptr as u64, 0, 0, 0, 0, 0, 0, 0]
    }

    fn dispatch_and_drain(rt: &mut Runtime, unit: UnitId, nid: u32, args: &[u64; 9]) -> u64 {
        let routed = dispatch(rt, unit, nid, args);
        assert_eq!(routed, Some(()), "NID {nid:#010x} dispatch returned None");
        rt.registry_mut()
            .drain_syscall_return(unit)
            .expect("dispatch should have set a syscall return")
    }

    #[test]
    fn create_lock_unlock_destroy_single_thread() {
        let (mut rt, unit, mutex_ptr) = lwmutex_runtime();
        let args = create_args(mutex_ptr);
        assert_eq!(
            dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_CREATE, &args),
            0
        );
        assert_eq!(rt.lv2_host().lwmutexes().len(), 1);

        let args = ptr_args(mutex_ptr);
        assert_eq!(
            dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_LOCK, &args),
            0
        );
        // The HLE wrapper takes the uncontended lock entirely in
        // user space (owner field at offset 0), so the kernel
        // side stays untouched: still signaled, no waiters parked.
        let owner_bytes = &rt.memory().as_bytes()[(mutex_ptr as usize)..(mutex_ptr as usize + 4)];
        let owner = u32::from_be_bytes([
            owner_bytes[0],
            owner_bytes[1],
            owner_bytes[2],
            owner_bytes[3],
        ]);
        assert_ne!(owner, LWMUTEX_FREE_OWNER);

        assert_eq!(
            dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_UNLOCK, &args),
            0
        );
        // Unlock cleared the user-space owner.
        let owner_bytes = &rt.memory().as_bytes()[(mutex_ptr as usize)..(mutex_ptr as usize + 4)];
        let owner = u32::from_be_bytes([
            owner_bytes[0],
            owner_bytes[1],
            owner_bytes[2],
            owner_bytes[3],
        ]);
        assert_eq!(owner, LWMUTEX_FREE_OWNER);

        assert_eq!(
            dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_DESTROY, &args),
            0
        );
        assert_eq!(rt.lv2_host().lwmutexes().len(), 0);
    }

    #[test]
    fn trylock_after_lock_returns_ebusy() {
        let (mut rt, unit, mutex_ptr) = lwmutex_runtime();
        dispatch_and_drain(
            &mut rt,
            unit,
            sys_nid::LWMUTEX_CREATE,
            &create_args(mutex_ptr),
        );
        let args = ptr_args(mutex_ptr);

        assert_eq!(
            dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_LOCK, &args),
            0
        );
        // try_acquire returns Contended on any held mutex regardless
        // of caller identity, mapped to CELL_EBUSY by the LV2 host.
        let trylock_ret = dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_TRYLOCK, &args);
        assert_eq!(
            trylock_ret as u32,
            cellgov_ps3_abi::cell_errors::CELL_EBUSY.code,
        );
    }

    #[test]
    fn destroy_while_held_returns_ebusy() {
        let (mut rt, unit, mutex_ptr) = lwmutex_runtime();
        dispatch_and_drain(
            &mut rt,
            unit,
            sys_nid::LWMUTEX_CREATE,
            &create_args(mutex_ptr),
        );
        let args = ptr_args(mutex_ptr);

        dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_LOCK, &args);
        let destroy_ret = dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_DESTROY, &args);
        assert_eq!(
            destroy_ret as u32,
            cellgov_ps3_abi::cell_errors::CELL_EBUSY.code,
        );
    }

    #[test]
    fn lock_on_unknown_id_returns_esrch() {
        let (mut rt, unit, mutex_ptr) = lwmutex_runtime();
        // Hand-write a fake lwmutex struct at mutex_ptr with a
        // never-allocated id at offset 0x10.
        let mut buf = [0u8; 24];
        buf[16..20].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        rt.memory_mut()
            .apply_commit(
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(mutex_ptr as u64), 24)
                    .unwrap(),
                &buf,
            )
            .unwrap();

        let lock_ret =
            dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_LOCK, &ptr_args(mutex_ptr));
        assert_eq!(
            lock_ret as u32,
            cellgov_ps3_abi::cell_errors::CELL_ESRCH.code,
        );
    }

    #[test]
    fn create_with_oob_attr_ptr_returns_efault() {
        let (mut rt, unit, mutex_ptr) = lwmutex_runtime();
        let args: [u64; 9] = [0, mutex_ptr as u64, 0xFFFF_FFFF, 0, 0, 0, 0, 0, 0];
        let ret = dispatch_and_drain(&mut rt, unit, sys_nid::LWMUTEX_CREATE, &args);
        assert_eq!(ret as u32, CELL_EFAULT.code);
    }
}
