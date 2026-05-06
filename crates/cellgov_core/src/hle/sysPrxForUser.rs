//! sysPrxForUser HLE implementations. Kernel-side `sc` trap
//! handlers live in `cellgov_lv2`.
//!
//! Bad guest pointers return `CELL_EFAULT`; loader/runtime
//! invariants use `debug_assert!`; oracle-state corruption (heap
//! or id-counter exhaustion) panics via `.expect`.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors::CELL_EFAULT;
use cellgov_ps3_abi::nid::sys_prx_for_user as sys_nid;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = sys_nid::OWNED;

/// Returns `None` if the NID is not owned here.
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
            // Lower bound widened below the configured 0xD0000000
            // stack base because PSL1GHT startup leaves locals
            // there before main() runs.
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
            // Bump allocator cannot release individual allocations.
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
            let table_id = runtime.lv2_host().ppu_thread_id_for_unit(source);
            // Unseeded table: every unit collapses to the fallback
            // id, colliding thread-id-keyed state across units.
            debug_assert!(
                table_id.is_some(),
                "sys_ppu_thread_get_id: LV2 thread table not seeded for unit {source:?}"
            );
            let id: u64 = table_id.map(|tid| tid.raw()).unwrap_or(0x0100_0000);
            let ptr = args[0] as u32;
            let mut ctx = adapter(runtime, source, nid);
            ctx.write_guest(ptr as u64, &id.to_be_bytes())
                .expect("sys_ppu_thread_get_id: write to caller out-ptr failed");
            ctx.set_return(0);
        }
        sys_nid::TIME_GET_SYSTEM_TIME => {
            // 1 tick = 1 ns; us = ticks / 1000.
            let us = runtime.time().raw() / 1_000;
            adapter(runtime, source, nid).set_return(us);
        }
        sys_nid::PPU_THREAD_CREATE => {
            // SDK-side wrapper signature (id_ptr, entry_opd, arg,
            // prio, stacksize, flags, name); takes the entry OPD
            // directly, unlike the LV2 syscall which receives a
            // param-struct pointer.
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
        pending_callback_spawn: &mut runtime.hle.pending_callback_spawn,
    }
}

pub(crate) fn initialize_tls(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let tls_seg_addr = args[2] as u32;
    let tls_seg_size = args[3] as u32;
    let tls_mem_size = args[4] as u32;

    // ELF PT_TLS invariant: p_filesz <= p_memsz.
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
    // Malformed p_filesz > p_memsz wraps to a huge value and is
    // rejected by the bounds check below.
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
    // libc memset faults on a bad page; the oracle has no faulting
    // path, so a failed write surfaces as CELL_EFAULT.
    let data = vec![val; size as usize];
    match ctx.write_guest(ptr as u64, &data) {
        Ok(()) => ctx.set_return(args[1]),
        Err(_) => ctx.set_return(CELL_EFAULT.into()),
    }
}

/// Allocates the `sleep_queue` id from the LV2 lwmutex table so the
/// matching `lock`/`unlock`/`trylock`/`destroy` calls resolve
/// through the same blocking surface.
pub(crate) fn lwmutex_create(runtime: &mut Runtime, source: UnitId, nid: u32, args: &[u64; 9]) {
    let mutex_ptr = args[1] as u32;
    let attr_ptr = args[2] as u32;

    // Region-aware read: guests pass stack-allocated attrs above
    // 0xD0000000, outside the linear user-memory region.
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

    // Stack garbage in waiter / recursive_count would mislead the
    // user-space fast path into spurious kernel calls.
    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
    buf[8..12].copy_from_slice(&(recursive | protocol).to_be_bytes());
    buf[16..20].copy_from_slice(&sleep_queue.to_be_bytes());

    let mut ctx = adapter(runtime, source, nid);
    match ctx.write_guest(mutex_ptr as u64, &buf) {
        Ok(()) => ctx.set_return(0),
        Err(_) => ctx.set_return(CELL_EFAULT.into()),
    }
}

/// Tracks live-object count for `sys_process_get_number_of_object`
/// and writes a non-zero handle marker; wait/signal semantics are
/// not modeled.
pub(crate) fn lwcond_create(runtime: &mut Runtime, source: UnitId, nid: u32, args: &[u64; 9]) {
    let lwcond_ptr = args[1] as u32;
    let _lwmutex_ptr = args[2] as u32;
    let _attr_ptr = args[3] as u32;

    runtime.lv2_host_mut().lwcond_count_inc();

    let id = 0xFFFFFFFFu32;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&id.to_be_bytes());
    let mut ctx = adapter(runtime, source, nid);
    let _ = ctx.write_guest(lwcond_ptr as u64, &buf);
    ctx.set_return(0);
}

/// Reads the embedded `sleep_queue` id at offset 0x10 and dispatches
/// the supplied request through the LV2 lwmutex surface.
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

    // Region-aware: stack-allocated structs above 0xD0000000.
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
//   +0  u32 owner            (0xFFFF_FFFF == free)
//   +4  u32 waiter           (kernel sleep-queue depth)
//   +8  u32 attribute        (recursive | protocol)
//   +12 u32 recursive_count
//   +16 u32 sleep_queue      (kernel-side id)
//   +20 u32 pad
const LWMUTEX_OFF_OWNER: u64 = 0;
const LWMUTEX_OFF_WAITER: u64 = 4;
const LWMUTEX_OFF_RECURSIVE_COUNT: u64 = 12;
const LWMUTEX_FREE_OWNER: u32 = 0xFFFF_FFFF;
const SYS_SYNC_RECURSIVE: u32 = 0x10;

struct LwMutexFields {
    owner: u32,
    waiter: u32,
    attribute: u32,
    recursive_count: u32,
    sleep_queue: u32,
}

/// `None` on a bad `mutex_ptr`.
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

fn write_lwmutex_u32(ctx: &mut dyn HleContext, mutex_ptr: u32, off: u64, value: u32) -> bool {
    ctx.write_guest((mutex_ptr as u64) + off, &value.to_be_bytes())
        .is_ok()
}

/// Truncated LV2 thread id used as the user-space owner field so
/// later calls recognise the same thread.
fn caller_owner_id(runtime: &Runtime, source: UnitId) -> u32 {
    runtime
        .lv2_host()
        .ppu_thread_id_for_unit(source)
        .map(|tid| tid.raw() as u32)
        .unwrap_or(0)
}

/// User-space fast path; only contention reaches the kernel.
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
        // Stale owner (thread exited without unlocking) is treated
        // as free so the mutex is not orphaned forever.
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
        // PSL1GHT's wrapper owns recursive_count bookkeeping;
        // returning OK lets it proceed.
        if (fields.attribute & SYS_SYNC_RECURSIVE) != 0 {
            adapter(runtime, source, nid).set_return(0);
            return;
        }
        adapter(runtime, source, nid).set_return(cellgov_ps3_abi::cell_errors::CELL_EDEADLK.into());
        return;
    }
    // Bump the user-space waiter so a concurrent unlocker invokes
    // the kernel; post-wake fields are filled in by the runtime
    // via `PendingResponse::LwMutexWake`.
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
}

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
    // recursive_count went 1 -> 0; lwmutex leaves the holder's
    // critical-section set.
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

/// Held check reads the user-space owner field; the kernel side
/// has no ownership tracking.
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

/// Never blocks; contention reports `CELL_EBUSY`.
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
    // Finished units never resume; the syscall-return slot doubles
    // as a post-mortem exit-code carrier. A registry change that
    // clears the slot on Finished units would drop the exit code.
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

    /// Minimum runtime that lets any sys-module NID reach its handler.
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

    /// A handler panic on synthetic-zero args counts as "routed":
    /// `catch_unwind` captures it as evidence of dispatch reaching
    /// the body.
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
                Err(_) => {}
            }
        }
    }

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
        // Never-allocated id at offset 0x10.
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
