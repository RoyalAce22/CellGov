//! `sys_process` dispatch handlers. Each method consumes the
//! [`crate::request::Lv2Request`] fields directly so the top-level
//! dispatch match stays a one-line delegation per arm.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

/// `sys_memory_access_right_raw_spu` flag value from `sys_memory.h`.
const SYS_MEMORY_ACCESS_RIGHT_RAW_SPU: u64 = 0x0000_0000_0000_0001;
/// `sys_memory_access_right_spu_thr` flag value from `sys_memory.h`.
const SYS_MEMORY_ACCESS_RIGHT_SPU_THR: u64 = 0x0000_0000_0000_0002;

impl Lv2Host {
    /// `sys_process_exit`: the kernel handler discards the requesting
    /// thread group; CellGov leaves termination to the runtime and
    /// reports CELL_OK so the calling unit's commit batch lands.
    pub(in crate::host) fn dispatch_process_exit(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0u64)
    }

    /// `sys_process_getpid`: CellGov hosts a single synthetic process;
    /// PSL1GHT tests rely on this spec PID constant.
    pub(in crate::host) fn dispatch_process_get_pid(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0x0100_0500)
    }

    /// `sys_process_getppid`: spec parent PID constant.
    pub(in crate::host) fn dispatch_process_get_ppid(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0x0100_0300)
    }

    /// `sys_process_get_ppu_guid`: matches the PPID constant; PSL1GHT
    /// keys on these values being equal for the synthetic single-
    /// process layout.
    pub(in crate::host) fn dispatch_process_get_ppu_guid(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0x0100_0300)
    }

    /// `sys_process_is_stack`: real LV2 reports 0 unless the address
    /// is one of the per-thread stack ranges; CellGov reports 0
    /// uniformly (no test in the matrix exercises a positive case).
    pub(in crate::host) fn dispatch_process_is_stack(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0u64)
    }

    /// `sys_process_is_spu_lock_line_reservation_address`. Mirrors LV2:
    /// the flags must be non-zero and only carry SPU_THR / RAW_SPU
    /// bits; the address's top nibble selects the verdict. CellGov
    /// does not track sys_mmapper regions, so unknown top nibbles
    /// fall through to CELL_EINVAL rather than RPCS3's vm-region
    /// lookup.
    pub(in crate::host) fn dispatch_process_is_spu_lock_line_reservation_address(
        &self,
        addr: u32,
        flags: u64,
    ) -> Lv2Dispatch {
        let known_bits = SYS_MEMORY_ACCESS_RIGHT_SPU_THR | SYS_MEMORY_ACCESS_RIGHT_RAW_SPU;
        if flags == 0 || (flags & !known_bits) != 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let code = match addr >> 28 {
            0x0 | 0x1 | 0x2 | 0xc | 0xe => 0u64,
            0xf => {
                if flags & SYS_MEMORY_ACCESS_RIGHT_RAW_SPU != 0 {
                    errno::CELL_EPERM.into()
                } else {
                    0
                }
            }
            0xd => errno::CELL_EPERM.into(),
            _ => errno::CELL_EINVAL.into(),
        };
        Lv2Dispatch::Immediate {
            code,
            effects: vec![],
        }
    }

    /// `sys_spu_initialize`. Real LV2 records per-process SPU limits
    /// in a kernel-side `spu_limits_t`. CellGov is the oracle, not a
    /// scheduler: it validates `max_raw_spu <= 5` (matches LV2 and
    /// RPCS3) and otherwise reports CELL_OK without persisting any
    /// state -- nothing in the runtime keys on the announced limits.
    pub(in crate::host) fn dispatch_spu_initialize(
        &self,
        _max_usable_spu: u32,
        max_raw_spu: u32,
    ) -> Lv2Dispatch {
        if max_raw_spu > 5 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// Per-class active-object count for
    /// `sys_process_get_number_of_object`. Maps `sys_process.h`
    /// `SYS_*_OBJECT` ids onto CellGov's tables; unmodeled classes
    /// report zero. Writes a 32-bit count (PS3 PPU64 ILP32:
    /// `size_t` is 4 bytes).
    pub(in crate::host) fn dispatch_process_get_number_of_object(
        &self,
        class_id: u32,
        count_out_ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        let count = self.process_counts.count_for_class(class_id, self);
        self.immediate_write_u32(count, count_out_ptr, source)
    }

    /// Writes `0xFFFFFFFF` -- the value real PS3 reports for
    /// PSL1GHT-built homebrew with no SDK version recorded.
    pub(in crate::host) fn dispatch_process_get_sdk_version(
        &self,
        version_out_ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        let version: u32 = 0xFFFF_FFFF;
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(version_out_ptr, 4),
            bytes: WritePayload::from_slice(&version.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// Writes the 64-byte SFO blob real PS3 returns for
    /// PSL1GHT-built homebrew with no PARAM.SFO loaded: version=1
    /// at byte 0, parental_level=4 at byte 23, attribute=1 at byte
    /// 31, rest zero.
    pub(in crate::host) fn dispatch_process_get_paramsfo(
        &self,
        buf_ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        let mut blob = [0u8; 64];
        blob[0] = 0x01;
        blob[23] = 0x04;
        blob[31] = 0x01;
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(buf_ptr, 64),
            bytes: WritePayload::from_slice(&blob),
            ordering: PriorityClass::Normal,
            source,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}
