//! `sys_rsx_memory_allocate` (665) and `sys_rsx_memory_free` (667).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// sys_rsx_memory_allocate (665). Bump-allocates `size` bytes
    /// and writes `mem_handle` (u32 BE) and `mem_addr` (u64 BE) into
    /// the guest out-pointers.
    ///
    /// # Errors
    ///
    /// `CELL_ENOMEM` if `size == 0`, the cursor would wrap, or the end
    /// address exceeds [`Lv2Host::SYS_RSX_MEM_END`].
    pub(in crate::host) fn dispatch_sys_rsx_memory_allocate(
        &mut self,
        mem_handle_ptr: u32,
        mem_addr_ptr: u32,
        size: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if size == 0 {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }
        let Some(end) = self.rsx_mem_alloc_ptr.checked_add(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        if end > Self::SYS_RSX_MEM_END {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }

        let handle = self.rsx_mem_handle_counter;
        let addr = self.rsx_mem_alloc_ptr;
        self.rsx_mem_alloc_ptr = end;
        self.rsx_mem_handle_counter = handle.wrapping_add(1);
        // Reserved slice a subsequent sys_rsx_context_allocate will consume
        // instead of bumping the cursor a second time.
        self.rsx_context.pending_mem_addr = addr;

        let handle_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(mem_handle_ptr, 4),
            bytes: WritePayload::from_slice(&handle.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        let addr_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(mem_addr_ptr, 8),
            bytes: WritePayload::from_slice(&(addr as u64).to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![handle_write, addr_write],
        }
    }

    /// sys_rsx_memory_free (667). No-op: the bump allocator never reclaims.
    pub(in crate::host) fn dispatch_sys_rsx_memory_free_noop(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::rsx::test_helpers::extract_write_u64;
    use crate::host::test_support::{extract_write_u32, FakeRuntime};
    use crate::request::Lv2Request;

    fn allocate_rsx(host: &mut Lv2Host, size: u32, source: UnitId) -> (u32, u64) {
        let rt = FakeRuntime::new(0x10_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr: 0x1000,
                mem_addr_ptr: 0x2000,
                size,
                flags: 0,
                a5: 0,
                a6: 0,
                a7: 0,
            },
            source,
            &rt,
        );
        match d {
            Lv2Dispatch::Immediate { code: 0, effects } => (
                extract_write_u32(&effects[0]),
                extract_write_u64(&effects[1]),
            ),
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    }

    #[test]
    fn sys_rsx_memory_allocate_returns_base_then_bumps() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);

        let (h1, a1) = allocate_rsx(&mut host, 0x30_0000, source);
        assert_eq!(h1, 1);
        assert_eq!(a1, Lv2Host::SYS_RSX_MEM_BASE as u64);

        let (h2, a2) = allocate_rsx(&mut host, 0x30_0000, source);
        assert_eq!(h2, 2);
        assert_eq!(a2, (Lv2Host::SYS_RSX_MEM_BASE + 0x30_0000) as u64);
    }

    #[test]
    fn sys_rsx_memory_allocate_rejects_zero_size() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr: 0x1000,
                mem_addr_ptr: 0x2000,
                size: 0,
                flags: 0,
                a5: 0,
                a6: 0,
                a7: 0,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(errno::CELL_ENOMEM)
        ));
    }

    #[test]
    fn sys_rsx_memory_allocate_rejects_beyond_region_end() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr: 0x1000,
                mem_addr_ptr: 0x2000,
                size: 0x2000_0000,
                flags: 0,
                a5: 0,
                a6: 0,
                a7: 0,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(errno::CELL_ENOMEM)
        ));
    }

    #[test]
    fn sys_rsx_memory_free_returns_ok() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxMemoryFree { mem_handle: 0xA001 },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code: 0, effects } if effects.is_empty()
        ));
    }
}
