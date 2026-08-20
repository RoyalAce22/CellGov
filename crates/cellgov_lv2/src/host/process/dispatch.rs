//! `sys_process` dispatch handlers.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

/// `sys_memory_access_right_raw_spu` flag value from `sys_memory.h`.
const SYS_MEMORY_ACCESS_RIGHT_RAW_SPU: u64 = 0x0000_0000_0000_0001;
/// `sys_memory_access_right_spu_thr` flag value from `sys_memory.h`.
const SYS_MEMORY_ACCESS_RIGHT_SPU_THR: u64 = 0x0000_0000_0000_0002;

/// Cap on marshalled-block pointer-table entries walked per list;
/// bounds the walk over an oversized block (the block itself already
/// bounds it for ordinary sizes).
const SPAWN_TABLE_MAX_ENTRIES: usize = 256;
/// Cap on a marshalled path/argv string read.
const SPAWN_STRING_MAX_LEN: usize = 1024;

impl Lv2Host {
    /// `sys_process_exit` from a boot-process unit reports CELL_OK so
    /// the calling unit's commit batch lands; termination is handled
    /// by the runtime. A unit bound to a spawned child instead
    /// finishes only that process.
    pub(in crate::host) fn dispatch_process_exit(&self, code: i32, source: UnitId) -> Lv2Dispatch {
        let pid = self.state.processes.process_of_unit(source);
        if pid == cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID {
            return Lv2Dispatch::immediate(0u64);
        }
        Lv2Dispatch::ProcessExitChild {
            pid,
            code,
            effects: vec![],
        }
    }

    /// `_sys_process_exit2`: exit carrying a `sys_exit2_param` block.
    ///
    /// The argv walk follows RPCS3 `sys_process.cpp _sys_process_exit2`
    /// (pointer array at param +0x28: argv strings, NULL, envp
    /// strings, NULL). Empty argv is a plain `sys_process_exit`.
    /// Non-empty argv requests exitspawn -- reboot into `argv[0]`
    /// with argv/envp/data carried over. The re-spawn itself is not
    /// modeled yet (the kernel-side spawn-request queue vsh's sc-23
    /// service consumes is the natural carrier; its record format is
    /// undecoded). The RPCS3 handoff semantics: LV2 memory
    /// containers survive the handoff with `used`
    /// reset to 0, and the default container's capacity can only
    /// DECREASE across exitspawn -- a higher SDK-suggested capacity
    /// is ignored, a lower one is honored, and capacity freed by the
    /// shrink may be spent on user containers
    /// (`lv2_exitspawn` in RPCS3 sys_process.cpp).
    pub(in crate::host) fn dispatch_process_exit2(
        &mut self,
        code: i32,
        arg_ptr: u32,
        arg_size: u32,
        source: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let argv0 = rt
            .read_committed(u64::from(arg_ptr) + 0x28, 8)
            .map(|b| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
            .and_then(|args_array| rt.read_committed(args_array, 8))
            .map(|b| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
        match argv0 {
            None => {
                // RPCS3 `sys_process.cpp _sys_process_exit2` reads the
                // param block unconditionally; an unreadable block is
                // a guest fault, not the empty-argv arm. Keep the
                // plain-exit outcome but do not decide it silently.
                self.log_invariant_break(
                    "process.exit2_param_unreadable",
                    format_args!(
                        "_sys_process_exit2 param block at {arg_ptr:#x} \
                         (arg_size={arg_size:#x}) unreadable; treating as \
                         plain process exit"
                    ),
                );
            }
            // Empty argv is the plain-exit arm (RPCS3 `sys_process.cpp`
            // `_sys_process_exit2` falls through to `_sys_process_exit`).
            Some(0) => {}
            Some(path_ptr) => {
                let path = rt
                    .read_committed_until(path_ptr, SPAWN_STRING_MAX_LEN, 0)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .unwrap_or_else(|| String::from("<unreadable>"));
                // arg_size > 0x1030 additionally carries a 0x1000-byte
                // data blob at the block's tail (RPCS3 `sys_process.cpp`
                // `_sys_process_exit2`); recorded here so the trace
                // shows what the unmodeled re-spawn dropped.
                self.log_invariant_break(
                    "process.exitspawn_not_modeled",
                    format_args!(
                        "_sys_process_exit2 with non-empty argv (argv[0]={path}, \
                         arg_size={arg_size:#x}); re-spawn via the kernel \
                         spawn-request queue is not modeled, treating as \
                         plain process exit"
                    ),
                );
            }
        }
        self.dispatch_process_exit(code, source)
    }

    /// `_sys_process_spawn` / `sys_process_spawns_a_self2`: parse the
    /// marshalled block, resolve the SELF image, mint the child's
    /// process entry.
    ///
    /// Block layout decoded from vsh 0x608950: `{ u64 table_off,
    /// u64, ptr table [8B entries], packed strings }`; the table is
    /// argv (argv[0] = SELF path), NULL, envp, NULL -- matching the
    /// pointer-walk shape RPCS3 uses on the exit2 side. Only argv[0]
    /// is consumed here; argv/envp delivery to the child's entry is
    /// not modeled yet.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::host) fn dispatch_process_spawn(
        &mut self,
        pid_out_ptr: u32,
        prio: i32,
        flags: u64,
        block_ptr: u32,
        block_size: u32,
        data_word: u64,
        source: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if flags != 0 {
            // The spawn flags word carries the child's primary-stack-size
            // selection (RPCS3 `sys_process.h`
            // `SYS_PROCESS_PRIMARY_STACK_SIZE_*`); the spawn loader
            // currently sizes the child stack itself, so a nonzero
            // request is dropped -- witnessed, never silent.
            self.log_invariant_break(
                "process.spawn_flags_not_modeled",
                format_args!(
                    "process spawn flags {flags:#x} not modeled; the child \
                     primary-stack-size request is dropped"
                ),
            );
        }
        if data_word != 0 {
            self.log_invariant_break(
                "process.spawn_data_word_not_modeled",
                format_args!(
                    "process spawn data word {data_word:#x} (sc 27 r8) not \
                     consumed; the child never observes it"
                ),
            );
        }
        if !rt.writable(u64::from(pid_out_ptr), 4) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let Some(table_off) = rt
            .read_committed(u64::from(block_ptr), 8)
            .map(|b| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
        else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        if table_off >= u64::from(block_size) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let table_base = u64::from(block_ptr) + table_off;
        // `block_size` is the caller-declared extent of the marshalled
        // block; the pointer table and its argv NULL terminator must
        // sit inside it, so the walk never reads bytes the caller did
        // not marshal.
        let in_block_entries = (u64::from(block_size) - table_off) / 8;
        let walk_limit = in_block_entries.min(SPAWN_TABLE_MAX_ENTRIES as u64);
        let mut path: Option<Vec<u8>> = None;
        let mut argv_terminated = false;
        for idx in 0..walk_limit {
            let Some(entry) = rt
                .read_committed(table_base + idx * 8, 8)
                .map(|b| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
            else {
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            };
            if entry == 0 {
                argv_terminated = true;
                break;
            }
            if idx == 0 {
                path = rt
                    .read_committed_until(entry, SPAWN_STRING_MAX_LEN, 0)
                    .map(|b| b.to_vec());
            }
        }
        if !argv_terminated {
            self.log_invariant_break(
                "process.spawn_table_unterminated",
                format_args!(
                    "spawn pointer table has no argv NULL terminator within \
                     the walked bound (block_size={block_size:#x}, \
                     table_off={table_off:#x}, walked={walk_limit}); rejecting"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let Some(path) = path else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let Some(record) = self.state.content.lookup_by_path(&path) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into());
        };
        let elf_bytes = record.elf_bytes.clone();
        let pid = self.state.processes.next_child_pid();
        let ppid = self.state.processes.process_of_unit(source);
        let inserted = self.state.processes.insert_child(
            pid,
            super::ProcessEntry {
                ppid,
                authority_id: cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID,
                control_flags1: 0,
                exit_status: None,
            },
        );
        if !inserted {
            // `next_child_pid` saturates at u32::MAX, so an occupied
            // pid means the mint space is exhausted. CellGov-decided
            // errno (RPCS3 todo-stubs the call): the resource-
            // exhaustion code, never a spawn against the occupant.
            self.log_invariant_break(
                "process.spawn_pid_space_exhausted",
                format_args!("next_child_pid returned occupied pid {pid:#x}; spawn rejected"),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EAGAIN.into());
        }
        Lv2Dispatch::ProcessSpawn {
            pid,
            pid_out_ptr,
            prio,
            path,
            elf_bytes,
            effects: vec![],
        }
    }

    /// `sys_process_get_status`: minimal liveness poll.
    ///
    /// CellGov-decided semantics: CELL_OK while `pid` names a live
    /// process, CELL_ESRCH once it has exited or never existed. The
    /// real LV2 encoding is undecoded (RPCS3 todo-stubs the call);
    /// revisit when a capture pins it.
    pub(in crate::host) fn dispatch_process_get_status(&self, pid: u32) -> Lv2Dispatch {
        let code = match self.state.processes.get(pid) {
            Some(entry) if entry.exit_status.is_none() => 0u64,
            _ => cell_errors::CELL_ESRCH.into(),
        };
        Lv2Dispatch::immediate(code)
    }

    /// `sys_process_getpid`: the calling process's pid, consistent
    /// with the pid the spawn wrote to the parent's `pid_out`.
    /// Unbound units are the boot process.
    pub(in crate::host) fn dispatch_process_get_pid(&self, source: UnitId) -> Lv2Dispatch {
        Lv2Dispatch::immediate(self.state.processes.process_of_unit(source).into())
    }

    /// `sys_process_getppid`: the calling process's parent pid -- for
    /// a spawned child, the pid of the process that spawned it.
    pub(in crate::host) fn dispatch_process_get_ppid(&mut self, source: UnitId) -> Lv2Dispatch {
        let pid = self.state.processes.process_of_unit(source);
        let ppid = match self.state.processes.get(pid) {
            Some(entry) => entry.ppid,
            None => {
                // Reachable only through a unit binding naming a pid
                // the table never held; the boot ppid served here is a
                // fabricated answer, so it never passes silently.
                self.log_invariant_break(
                    "process.ppid_of_unknown_pid",
                    format_args!(
                        "getppid from {source:?} bound to pid {pid:#x} with \
                         no table entry; serving the boot ppid"
                    ),
                );
                cellgov_ps3_abi::sys_process::BOOT_PROCESS_PPID
            }
        };
        Lv2Dispatch::immediate(ppid.into())
    }

    /// `sys_process_get_ppu_guid`: equals the boot ppid (PSL1GHT
    /// keys on the equality).
    pub(in crate::host) fn dispatch_process_get_ppu_guid(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(self.state.processes.boot().ppid.into())
    }

    /// `sys_process_is_stack`: 1 when `addr` is in any tracked PPU
    /// thread's `[stack_base, stack_base + stack_size)`, else 0.
    pub(in crate::host) fn dispatch_process_is_stack(&self, addr: u32) -> Lv2Dispatch {
        let on_stack = self.state.ppu_threads.iter_ids().any(|tid| {
            let attrs = match self.state.ppu_threads.get(tid) {
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
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let count = self.state.process_counts.count_for_class(class_id, self);
        self.immediate_write_u32(count, count_out_ptr, source, tick)
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
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let version: u32 = self.sdk_version();
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(version_out_ptr, 4),
            bytes: WritePayload::from_slice(&version.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source,
            source_time: tick,
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
        tick: GuestTicks,
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
            source_time: tick,
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
