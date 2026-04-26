//! `sys_memory_allocate` bump-allocator dispatch.
//!
//! The rest of the `sys_memory_*` family is inlined at the dispatch
//! match because each is a single arithmetic expression.

use cellgov_event::UnitId;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

/// End (exclusive) of the PS3 `main` region the LV2 allocator may
/// hand out from. Shared with ELF PT_LOAD, TLS, and the HLE bump
/// arena; the bound catches runaways before they walk into unmapped
/// space.
const MEM_ALLOC_REGION_END: u32 = 0x4000_0000;

impl Lv2Host {
    pub(super) fn dispatch_memory_allocate(
        &mut self,
        size: u64,
        alloc_addr_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // 64KB alignment, every arithmetic step checked: a
        // u32-truncating size, an alignment wrap, or a cursor past
        // MEM_ALLOC_REGION_END all return CELL_ENOMEM with the
        // cursor unchanged. Silent wrap would hand out addresses
        // outside user memory.
        const ALIGN: u32 = 0x1_0000;
        let Ok(size) = u32::try_from(size) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        };
        let Some(aligned_ptr) = self
            .mem_alloc_ptr
            .checked_add(ALIGN - 1)
            .map(|p| p & !(ALIGN - 1))
        else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        };
        let Some(next) = aligned_ptr.checked_add(size) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        };
        if next > MEM_ALLOC_REGION_END {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        }
        self.mem_alloc_ptr = next;
        self.immediate_write_u32(aligned_ptr, alloc_addr_ptr, requester)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::Lv2Dispatch;
    use crate::host::test_support::{extract_write_u32, FakeRuntime};
    use crate::request::Lv2Request;

    #[test]
    fn memory_allocate_returns_aligned_sequential_addresses() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);

        let addr1 = match host.dispatch(
            Lv2Request::MemoryAllocate {
                size: 0x10000,
                flags: 0x200,
                alloc_addr_ptr: 0x100,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let addr2 = match host.dispatch(
            Lv2Request::MemoryAllocate {
                size: 0x10000,
                flags: 0x200,
                alloc_addr_ptr: 0x104,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };

        assert_eq!(addr1 & 0xFFFF, 0, "addr1 not 64KB-aligned");
        assert_eq!(addr2 & 0xFFFF, 0, "addr2 not 64KB-aligned");
        assert!(
            addr2 >= addr1 + 0x10000,
            "allocations overlap: 0x{addr1:x} and 0x{addr2:x}"
        );
    }

    #[test]
    fn set_mem_alloc_base_overrides_first_allocation_address() {
        let mut host = Lv2Host::new();
        host.set_mem_alloc_base(0x008A_0000);
        let rt = FakeRuntime::new(0x10000);
        let addr = match host.dispatch(
            Lv2Request::MemoryAllocate {
                size: 0x10000,
                flags: 0x200,
                alloc_addr_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        assert_eq!(
            addr, 0x008A_0000,
            "first allocation must use configured base"
        );
        assert_eq!(addr & 0xFFFF, 0, "alignment must be preserved");
    }

    #[test]
    fn memory_free_is_noop_stub() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::MemoryFree { addr: 0x0001_0000 },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![]
            }
        );
    }

    #[test]
    fn memory_get_user_memory_size_writes_info_struct() {
        // sys_memory_info_t layout: total_user_memory,
        // available_user_memory (both big-endian u32).
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);

        let result = host.dispatch(
            Lv2Request::MemoryGetUserMemorySize {
                mem_info_ptr: 0x200,
            },
            source,
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code: 0, effects } => {
                assert_eq!(effects.len(), 1, "expect one 8-byte write");
                match &effects[0] {
                    cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                        assert_eq!(range.start().raw(), 0x200);
                        assert_eq!(range.length(), 8);
                        let b = bytes.bytes();
                        let total = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                        let avail = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
                        assert_eq!(total, crate::CELL_PS3_USER_MEMORY_TOTAL);
                        assert_eq!(avail, crate::CELL_PS3_USER_MEMORY_TOTAL);
                    }
                    other => panic!("expected SharedWriteIntent, got {other:?}"),
                }
            }
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    }

    #[test]
    fn memory_container_create_writes_monotonic_id() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);

        let id1 = match host.dispatch(
            Lv2Request::MemoryContainerCreate {
                cid_ptr: 0x100,
                size: 0x10_0000,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let id2 = match host.dispatch(
            Lv2Request::MemoryContainerCreate {
                cid_ptr: 0x104,
                size: 0x10_0000,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        assert_ne!(id1, 0);
        assert_ne!(id1, id2, "IDs must be monotonic across create calls");
    }
}
