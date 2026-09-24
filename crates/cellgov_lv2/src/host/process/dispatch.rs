//! `sys_process` dispatch handlers.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::Lv2Dispatch;
use crate::host::guest_struct::read_be_u64;
use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

/// `sys_memory_access_right_raw_spu` flag bit.
const SYS_MEMORY_ACCESS_RIGHT_RAW_SPU: u64 = 0x0000_0000_0000_0001;
/// `sys_memory_access_right_spu_thr` flag bit.
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
    /// finishes only that process. The runtime cascades Finished to
    /// every unit of the process that exits and records a child's exit
    /// status for `sys_process_get_status` polls. A child's exit leaves
    /// the boot process untouched.
    pub(in crate::host) fn dispatch_process_exit(&self, code: i32, source: UnitId) -> Lv2Dispatch {
        let pid = self.state.processes.process_of_unit(source);
        if pid == cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID {
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
    /// The argv walk follows the block liblv2's
    /// `sys_game_process_exitspawn` builds before it issues sc 26; its
    /// layout is [`cellgov_ps3_abi::lv2::process::exit2_param`]. Empty
    /// argv is a plain `sys_process_exit`. Non-empty argv
    /// requests exitspawn -- reboot into `argv[0]` with argv/envp/data
    /// carried over. The re-spawn itself is not modeled yet (the
    /// kernel-side spawn-request queue vsh's sc-23 service consumes is
    /// the natural carrier; its record format is undecoded). One
    /// handoff rule is settled: a memory container the process holds
    /// is released on an ordinary exit but survives an exitspawn. How
    /// the default container's capacity is renegotiated across the
    /// handoff is unestablished.
    pub(in crate::host) fn dispatch_process_exit2(
        &mut self,
        code: i32,
        arg_ptr: u32,
        arg_size: u32,
        source: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let argv_slot = u64::from(arg_ptr)
            + u64::from(cellgov_ps3_abi::lv2::process::exit2_param::ARGV_ARRAY_OFFSET);
        let argv0 = read_be_u64(rt, argv_slot).and_then(|args_array| read_be_u64(rt, args_array));
        match argv0 {
            None => {
                // The param block is read unconditionally, so an
                // unreadable one is a guest fault, not the empty-argv
                // arm. Keep the plain-exit outcome but do not decide
                // it silently.
                self.log_invariant_break(
                    "process.exit2_param_unreadable",
                    format_args!(
                        "_sys_process_exit2 param block at {arg_ptr:#x} \
                         (arg_size={arg_size:#x}) unreadable; treating as \
                         plain process exit"
                    ),
                );
            }
            // Empty argv is the plain-exit arm: nothing to reboot
            // into, so this degenerates to `sys_process_exit`.
            Some(0) => {}
            Some(path_ptr) => {
                let path = rt
                    .read_committed_until(path_ptr, SPAWN_STRING_MAX_LEN, 0)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .unwrap_or_else(|| String::from("<unreadable>"));
                // An `arg_size` past `exit2_param::DATA_BLOB_THRESHOLD`
                // additionally carries the caller's data blob at the
                // block's tail. Recorded here so the trace shows what
                // the unmodeled re-spawn dropped.
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
    /// argv (`argv[0]` = SELF path), NULL, envp, NULL.
    ///
    /// [CBE-Handbook p:397 s:14.3.1.3] The OS hands a program an
    /// argument-pointer array and an environment-pointer array, each
    /// terminated by a NULL pointer.
    ///
    /// Only `argv[0]` is consumed here; argv/envp delivery to the
    /// child's entry is not modeled yet.
    ///
    /// `block_size` and [`SPAWN_TABLE_MAX_ENTRIES`] bound the
    /// pointer-table walk. The path resolves through the content
    /// store. The runtime hands the image to the injected spawn
    /// loader, which installs it into a fresh child address space.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when `pid_out_ptr` is not writable, or the
    ///   block's table offset or a table entry is unreadable.
    /// - `CELL_EFAULT` when `table_off` is at or past `block_size`.
    /// - `CELL_EFAULT` when the walked bound holds no argv terminator;
    ///   a named break records it.
    /// - `CELL_EFAULT` when argv is empty or `argv[0]` is unreadable.
    /// - `CELL_ENOENT` when the content store holds no image at
    ///   `argv[0]`.
    /// - `CELL_EAGAIN` when the child pid space is exhausted.
    /// - `CELL_EFAULT` from the runtime when the spawn loader refuses
    ///   the image. The runtime unwinds the pid this arm minted and
    ///   the child space under a
    ///   `runtime.process_spawn_image_load_failed` break.
    /// - `CELL_ENOSYS` from the runtime when no spawn loader or PPU
    ///   factory is installed.
    /// - `CELL_ENOMEM` from the runtime when no address-space id is
    ///   free for the child.
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
            // The spawn flags word carries the child's
            // primary-stack-size selection; the spawn loader currently
            // sizes the child stack itself, so a nonzero request is
            // dropped -- witnessed, never silent.
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
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let Some(table_off) = read_be_u64(rt, u64::from(block_ptr)) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        if table_off >= u64::from(block_size) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
            let Some(entry) = read_be_u64(rt, table_base + idx * 8) else {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let Some(path) = path else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let Some(record) = self.state.content.lookup_by_path(&path) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOENT.into());
        };
        let elf_bytes = record.elf_bytes.clone();
        let pid = self.state.processes.next_child_pid();
        let ppid = self.state.processes.process_of_unit(source);
        let inserted = self.state.processes.insert_child(
            pid,
            super::ProcessEntry {
                ppid,
                authority_id: cellgov_ps3_abi::format::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID,
                control_flags1: 0,
                exit_status: None,
            },
        );
        if !inserted {
            // `next_child_pid` saturates at u32::MAX, so an occupied
            // pid means the mint space is exhausted. Nothing pins what
            // the kernel returns here, so the errno is CellGov's
            // choice: the resource-exhaustion code, never a spawn
            // against the occupant.
            self.log_invariant_break(
                "process.spawn_pid_space_exhausted",
                format_args!("next_child_pid returned occupied pid {pid:#x}; spawn rejected"),
            );
            return Lv2Dispatch::immediate(errno::CELL_EAGAIN.into());
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
    /// CELL_OK while `pid` names a live process, CELL_ESRCH after it
    /// exits or when it never existed.
    ///
    /// Known divergence: the status comes back as the syscall's return
    /// value, and firmware re-polls while it reads 1 or 2. Neither
    /// CELL_OK nor CELL_ESRCH means "not finished", so a guest wait
    /// loop on this call leaves on the first poll.
    pub(in crate::host) fn dispatch_process_get_status(&self, pid: u32) -> Lv2Dispatch {
        let code = match self.state.processes.get(pid) {
            Some(entry) if entry.exit_status.is_none() => 0u64,
            _ => errno::CELL_ESRCH.into(),
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
                cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PPID
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
    /// - `0x0`, `0x1`, `0x2`, `0xC` and `0xE` (main memory, user, RSX
    ///   and RawSPU MMIO): CELL_OK.
    /// - `0xD` (PPU stack): CELL_EPERM.
    /// - `0xF` (private SPU MMIO): CELL_EPERM under the RAW_SPU flag,
    ///   CELL_OK otherwise.
    /// - Any other nibble: CELL_EINVAL. A verdict there needs the
    ///   per-region sys_vm / sys_mmapper state CellGov does not track.
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

    /// `sys_spu_initialize`: validates `max_raw_spu <= 5` (LV2 cap).
    ///
    /// The arm persists no limit and partitions no SPU pool into
    /// usable and raw slots. It logs an invariant break, so a caller
    /// that reads the limits back shows in the trace.
    pub(in crate::host) fn dispatch_spu_initialize(
        &mut self,
        _max_usable_spu: u32,
        max_raw_spu: u32,
    ) -> Lv2Dispatch {
        if max_raw_spu > 5 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
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
    /// (`SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN`), the value a process
    /// param carries when it declares no SDK version. PSL1GHT-built
    /// homebrew leaves that word behind, and a console answers it for
    /// such a title in `tests/ps3autotests/tests/lv2/sys_process`.
    pub(in crate::host) fn dispatch_process_get_sdk_version(
        &self,
        version_out_ptr: u32,
        source: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let version: u32 = self.sdk_version();
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(version_out_ptr, 4),
            WritePayload::from_slice(&version.to_be_bytes()),
            source,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_process_get_paramsfo`: writes the 64-byte SFO blob a PS3
    /// returns for homebrew with no PARAM.SFO.
    ///
    /// `tests/ps3autotests/tests/lv2/sys_process` prints the whole
    /// buffer byte by byte for such a title:
    ///
    /// - `version` = 1 at offset 0
    /// - `parental_level` = 4 at offset 23
    /// - `attribute` = 1 at offset 31
    /// - every other byte zero
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
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(buf_ptr, 64),
            WritePayload::from_slice(&blob),
            source,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}

#[cfg(test)]
#[path = "tests/dispatch_tests.rs"]
mod tests;
