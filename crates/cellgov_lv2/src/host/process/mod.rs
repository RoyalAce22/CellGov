//! `sys_process` dispatch handlers. Each method consumes the
//! [`Lv2Request`] fields directly so the top-level dispatch match
//! stays a one-line delegation per arm.

mod counts;

pub(in crate::host) use counts::ProcessCounts;

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;

use super::Lv2Host;
use crate::dispatch::Lv2Dispatch;

impl Lv2Host {
    /// `sys_process_exit`: the kernel handler discards the requesting
    /// thread group; CellGov leaves termination to the runtime and
    /// reports CELL_OK so the calling unit's commit batch lands.
    pub(in crate::host) fn dispatch_process_exit(&self) -> Lv2Dispatch {
        Lv2Dispatch::Immediate {
            code: 0u64,
            effects: vec![],
        }
    }

    /// `sys_process_getpid`: CellGov hosts a single synthetic process;
    /// PSL1GHT tests rely on this spec PID constant.
    pub(in crate::host) fn dispatch_process_get_pid(&self) -> Lv2Dispatch {
        Lv2Dispatch::Immediate {
            code: 0x0100_0500,
            effects: vec![],
        }
    }

    /// `sys_process_getppid`: spec parent PID constant.
    pub(in crate::host) fn dispatch_process_get_ppid(&self) -> Lv2Dispatch {
        Lv2Dispatch::Immediate {
            code: 0x0100_0300,
            effects: vec![],
        }
    }

    /// `sys_process_get_ppu_guid`: matches the PPID constant; PSL1GHT
    /// keys on these values being equal for the synthetic single-
    /// process layout.
    pub(in crate::host) fn dispatch_process_get_ppu_guid(&self) -> Lv2Dispatch {
        Lv2Dispatch::Immediate {
            code: 0x0100_0300,
            effects: vec![],
        }
    }

    /// `sys_process_is_stack`: real LV2 reports 0 unless the address
    /// is one of the per-thread stack ranges; CellGov reports 0
    /// uniformly (no test in the matrix exercises a positive case).
    pub(in crate::host) fn dispatch_process_is_stack(&self) -> Lv2Dispatch {
        Lv2Dispatch::Immediate {
            code: 0u64,
            effects: vec![],
        }
    }

    /// Per-class active-object count for
    /// `sys_process_get_number_of_object`. Maps `sys_process.h`
    /// `SYS_*_OBJECT` ids onto CellGov's tables; unmodeled classes
    /// report zero. Writes a 32-bit count (PSL1GHT `size_t` is 4
    /// bytes in PPU64 ILP32).
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
