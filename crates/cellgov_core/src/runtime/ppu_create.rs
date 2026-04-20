//! PPU thread creation handler extracted from `runtime.rs`.
//!
//! Walks the four steps of `sys_ppu_thread_create`:
//!
//! 1. Resolve the OPD in guest memory (entry code + TOC, 16 BE bytes).
//! 2. Construct the child execution unit via the installed PPU
//!    factory and commit its initial TLS image.
//! 3. Register the child in the LV2 PPU thread table.
//! 4. Write the minted thread id into the caller's out pointer and
//!    return CELL_OK.
//!
//! Any step that fails short-circuits with a defensive return code
//! (`CELL_EFAULT`, `CELL_ENOSYS`, `CELL_EAGAIN`). Broken out to its
//! own module because the body is ~100 lines and is the most
//! likely spot for future ABI edge-case work.

use cellgov_event::UnitId;
use cellgov_lv2::{Lv2Dispatch, PpuThreadAttrs, PpuThreadInitState};

use super::Runtime;

impl Runtime {
    pub(super) fn handle_ppu_thread_create(&mut self, source: UnitId, dispatch: Lv2Dispatch) {
        let (
            id_ptr,
            entry_opd,
            stack_top,
            stack_base,
            stack_size,
            arg,
            tls_base,
            tls_bytes,
            priority,
        ) = match dispatch {
            Lv2Dispatch::PpuThreadCreate {
                id_ptr,
                entry_opd,
                stack_top,
                stack_base,
                stack_size,
                arg,
                tls_base,
                tls_bytes,
                priority,
                effects,
            } => {
                self.apply_lv2_effects(&effects);
                (
                    id_ptr, entry_opd, stack_top, stack_base, stack_size, arg, tls_base, tls_bytes,
                    priority,
                )
            }
            other => unreachable!("handle_ppu_thread_create called with {other:?}"),
        };
        // Resolve the OPD: entry_code is the first 8 bytes of the
        // OPD, TOC is the next 8. Big-endian.
        let opd_bytes =
            match cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(entry_opd as u64), 16)
                .and_then(|r| self.memory.read(r))
            {
                Some(b) if b.len() == 16 => {
                    let mut arr = [0u8; 16];
                    arr.copy_from_slice(b);
                    arr
                }
                _ => {
                    // Bad OPD address; return CELL_EFAULT (0x8001000e)
                    // to the caller and do not spawn a thread.
                    self.registry.set_syscall_return(source, 0x8001_000e);
                    return;
                }
            };
        let entry_code = u64::from_be_bytes(opd_bytes[0..8].try_into().unwrap());
        let entry_toc = u64::from_be_bytes(opd_bytes[8..16].try_into().unwrap());

        // Register the child unit via the PPU factory. Without a
        // factory we cannot construct a concrete PpuExecutionUnit
        // here (cellgov_core does not depend on cellgov_ppu), so
        // thread creation fails cleanly with ENOSYS.
        let Some(factory) = self.ppu_factory.as_ref() else {
            self.registry.set_syscall_return(source, 0x8001_0028);
            return;
        };
        let init = PpuThreadInitState {
            entry_code,
            entry_toc,
            arg,
            stack_top,
            tls_base,
            // LR sentinel: zero for now. When the child's entry
            // function returns, execution jumps to PC=0; the
            // interpreter faults cleanly and the unit ends.
            // Well-behaved guest code calls sys_ppu_thread_exit
            // explicitly and never reaches the sentinel.
            lr_sentinel: 0,
        };
        let child_unit_id = self
            .registry
            .register_dynamic(&|id| factory(id, init.clone()));

        // Commit TLS bytes into guest memory at tls_base.
        if !tls_bytes.is_empty() && tls_base != 0 {
            self.commit_bytes_at(tls_base, &tls_bytes);
        }

        // Register the child in the PPU thread table. Fails only
        // on u64 id exhaustion, which cannot happen here.
        let attrs = PpuThreadAttrs {
            entry: entry_opd as u64,
            arg,
            stack_base: stack_base as u32,
            stack_size: stack_size as u32,
            priority,
            tls_base: tls_base as u32,
        };
        let Some(thread_id) = self.lv2_host.ppu_threads_mut().create(child_unit_id, attrs) else {
            self.registry.set_syscall_return(source, 0x8001_0004);
            return;
        };

        // Write the minted thread id into the caller's output
        // pointer as a big-endian u64.
        self.commit_bytes_at(id_ptr as u64, &thread_id.raw().to_be_bytes());

        // Return CELL_OK to the caller.
        self.registry.set_syscall_return(source, 0);
    }
}
