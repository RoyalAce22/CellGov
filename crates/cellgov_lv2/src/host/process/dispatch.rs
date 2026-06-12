//! `sys_process` dispatch handlers.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

/// `sys_memory_access_right_raw_spu` flag value from `sys_memory.h`.
const SYS_MEMORY_ACCESS_RIGHT_RAW_SPU: u64 = 0x0000_0000_0000_0001;
/// `sys_memory_access_right_spu_thr` flag value from `sys_memory.h`.
const SYS_MEMORY_ACCESS_RIGHT_SPU_THR: u64 = 0x0000_0000_0000_0002;

impl Lv2Host {
    /// `sys_process_exit`: reports CELL_OK so the calling unit's
    /// commit batch lands; termination is handled by the runtime.
    pub(in crate::host) fn dispatch_process_exit(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0u64)
    }

    /// `sys_process_getpid`: spec PID constant for the single
    /// synthetic process.
    pub(in crate::host) fn dispatch_process_get_pid(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0x0100_0500)
    }

    /// `sys_process_getppid`: spec parent PID constant.
    pub(in crate::host) fn dispatch_process_get_ppid(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0x0100_0300)
    }

    /// `sys_process_get_ppu_guid`: equals the PPID constant (PSL1GHT
    /// keys on the equality).
    pub(in crate::host) fn dispatch_process_get_ppu_guid(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0x0100_0300)
    }

    /// `sys_process_is_stack`: 1 when `addr` is in any tracked PPU
    /// thread's `[stack_base, stack_base + stack_size)`, else 0.
    pub(in crate::host) fn dispatch_process_is_stack(&self, addr: u32) -> Lv2Dispatch {
        let on_stack = self.ppu_threads.iter_ids().any(|tid| {
            let attrs = match self.ppu_threads.get(tid) {
                Some(t) => &t.attrs,
                None => return false,
            };
            let end = attrs.stack_base.saturating_add(attrs.stack_size);
            (attrs.stack_base..end).contains(&addr)
        });
        Lv2Dispatch::immediate(if on_stack { 1 } else { 0 })
    }

    /// `sys_process_is_spu_lock_line_reservation_address`: flags must
    /// be non-zero and only carry SPU_THR / RAW_SPU bits; the address's
    /// top nibble selects the verdict.
    ///
    /// Unknown top nibbles return CELL_EINVAL (sys_mmapper regions
    /// are not tracked).
    pub(in crate::host) fn dispatch_process_is_spu_lock_line_reservation_address(
        &self,
        addr: u32,
        flags: u64,
    ) -> Lv2Dispatch {
        let known_bits = SYS_MEMORY_ACCESS_RIGHT_SPU_THR | SYS_MEMORY_ACCESS_RIGHT_RAW_SPU;
        if flags == 0 || (flags & !known_bits) != 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let code = match addr >> 28 {
            0x0 | 0x1 | 0x2 | 0xc | 0xe => 0u64,
            0xf => {
                if flags & SYS_MEMORY_ACCESS_RIGHT_RAW_SPU != 0 {
                    cell_errors::CELL_EPERM.into()
                } else {
                    0
                }
            }
            0xd => cell_errors::CELL_EPERM.into(),
            _ => cell_errors::CELL_EINVAL.into(),
        };
        Lv2Dispatch::Immediate {
            code,
            effects: vec![],
        }
    }

    /// `sys_spu_initialize`: validates `max_raw_spu <= 5` (LV2 cap).
    ///
    /// Announced limits are not persisted; an invariant-break is
    /// logged so any caller that reads them back is visible in the
    /// trace.
    pub(in crate::host) fn dispatch_spu_initialize(
        &mut self,
        _max_usable_spu: u32,
        max_raw_spu: u32,
    ) -> Lv2Dispatch {
        if max_raw_spu > 5 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        self.log_invariant_break(
            "dispatch.spu_initialize_limits_unpersisted",
            format_args!(
                "sys_spu_initialize: announced limits not persisted; \
                 max_usable_spu={_max_usable_spu} max_raw_spu={max_raw_spu}"
            ),
        );
        Lv2Dispatch::immediate(0)
    }

    /// `sys_process_get_number_of_object`: writes the per-class active
    /// count as a 32-bit value (PS3 PPU64 ILP32). Unmodeled classes
    /// report zero.
    pub(in crate::host) fn dispatch_process_get_number_of_object(
        &self,
        class_id: u32,
        count_out_ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        let count = self.process_counts.count_for_class(class_id, self);
        self.immediate_write_u32(count, count_out_ptr, source)
    }

    /// `sys_process_get_sdk_version`: writes the title's recorded
    /// SDK version. The value is read from the title ELF's
    /// `process_param_t` at boot
    /// (`cellgov_ppu::loader::find_sys_process_param`) and plumbed
    /// through via [`Lv2Host::set_sdk_version`]. Callers that never
    /// invoke the setter retain `0xFFFFFFFF`
    /// (`SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN`) -- the PS3
    /// absent-case sentinel for PSL1GHT homebrew. RPCS3 mirrors the
    /// same field at `sys_process.cpp`
    /// (`g_ps3_process_info.sdk_ver`, populated from the LOOS+1
    /// program header at `PPUModule.cpp`).
    pub(in crate::host) fn dispatch_process_get_sdk_version(
        &self,
        version_out_ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        let version: u32 = self.sdk_version();
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

    /// `sys_process_get_paramsfo`: writes the 64-byte SFO blob real
    /// PS3 returns for PSL1GHT homebrew with no PARAM.SFO.
    ///
    /// Layout: version=1@0, parental_level=4@23, attribute=1@31,
    /// rest zero.
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

#[cfg(test)]
#[path = "tests/dispatch_tests.rs"]
mod tests;
