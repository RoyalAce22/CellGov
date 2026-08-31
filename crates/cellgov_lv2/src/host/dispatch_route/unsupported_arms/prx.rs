//! `_sys_prx_*` module-lifecycle arms.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::{Lv2Host, Lv2Runtime};

use super::be::{read_be_u32, read_be_u64};

impl Lv2Host {
    /// `_sys_prx_start_module` (481): the two-phase start handshake.
    ///
    /// `pOpt->cmd & 0xF` selects the phase:
    ///
    /// 1. Phase 1 hands the caller the entry to invoke. CellGov runs
    ///    every firmware module's `module_start` itself at boot, so
    ///    this phase reports `NO_ENTRY`.
    /// 2. Phase 2 reports what that entry returned. A report of
    ///    `SYS_PRX_RESIDENT` marks the module started, and a later
    ///    unload then answers `NOT_REMOVABLE`.
    ///
    /// `pOpt->size` is never validated. The arm compares it against
    /// `MIN_SIZE` only to decide whether `entry2` exists. liblv2.sprx
    /// always declares the extended `0x28` form, so a firmware caller
    /// always has both slots.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for null `id` or null `pOpt`.
    /// - `CELL_ESRCH` when `id` names no loaded module.
    /// - `CELL_PRX_ERROR_ERROR` for a command nibble that is neither
    ///   1 nor 2.
    /// - `CELL_EFAULT` when `pOpt` is unreadable or the struct would
    ///   not fit inside the 32-bit guest address space.
    pub(in crate::host::dispatch_route) fn dispatch_prx_start_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::sys_prx::{
            start_cmd, start_stop_option as opt, CELL_PRX_ERROR_ERROR, SYS_PRX_RESIDENT,
        };

        let id = args[0] as u32;
        let p_opt = args[2] as u32;
        if id == 0 || p_opt == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        if self.state.prx_registry.lookup_by_id(id).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        // The base struct (through `res`) must fit below the 4 GiB
        // boundary; `entry2`'s reach is gated after `size` is known.
        if p_opt.checked_add(opt::MIN_SIZE as u32).is_none() {
            self.log_invariant_break(
                "dispatch.prx_start_module_p_opt_wraps",
                format_args!(
                    "_sys_prx_start_module option struct at p_opt={p_opt:#010x} wraps u32; \
                     returning CELL_EFAULT (struct does not fit in 32-bit guest address space)"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        let Some(size) = self.read_be_u64(rt, p_opt + opt::SIZE_OFFSET) else {
            self.log_invariant_break(
                "dispatch.prx_start_module_size_unreadable",
                format_args!(
                    "_sys_prx_start_module pOpt={p_opt:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        // Extended struct: entry2 at +0x20 must also fit.
        if size != opt::MIN_SIZE && p_opt.checked_add(opt::ENTRY2_OFFSET + 8).is_none() {
            self.log_invariant_break(
                "dispatch.prx_start_module_entry2_wraps",
                format_args!(
                    "_sys_prx_start_module extended option struct at p_opt={p_opt:#010x} \
                     (size={size:#x}) wraps u32 at entry2; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let Some(cmd) = self.read_be_u64(rt, p_opt + opt::CMD_OFFSET) else {
            self.log_invariant_break(
                "dispatch.prx_start_module_cmd_unreadable",
                format_args!(
                    "_sys_prx_start_module pOpt={p_opt:#010x} cmd field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };

        match cmd & start_cmd::MASK {
            start_cmd::GET_ENTRY => {
                let mut effects = vec![self.write_be_u64(
                    requester,
                    p_opt + opt::ENTRY_OFFSET,
                    opt::NO_ENTRY,
                    tick,
                )];
                // entry2 exists only in the extended struct.
                if size != opt::MIN_SIZE {
                    effects.push(self.write_be_u64(
                        requester,
                        p_opt + opt::ENTRY2_OFFSET,
                        opt::NO_ENTRY,
                        tick,
                    ));
                }
                Lv2Dispatch::Immediate { code: 0, effects }
            }
            start_cmd::REPORT_RESULT => {
                let Some(res) = self.read_be_u64(rt, p_opt + opt::RES_OFFSET) else {
                    self.log_invariant_break(
                        "dispatch.prx_start_module_res_unreadable",
                        format_args!(
                            "_sys_prx_start_module pOpt={p_opt:#010x} res field unreadable; \
                             returning CELL_EFAULT"
                        ),
                    );
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                if res == SYS_PRX_RESIDENT {
                    // LV2's STARTING -> STARTED transition: the module
                    // is now resident and refuses unload.
                    self.state.prx_registry.mark_started(id);
                    return Lv2Dispatch::immediate(0);
                }
                // Phase 1 handed back NO_ENTRY. liblv2.sprx reports
                // res = 0 when it finds neither entry callable, so no
                // firmware caller arrives here with another value.
                // `res` carries the sign-extended s32 a module_start
                // returned, so its low 32 bits are that code.
                self.log_invariant_break(
                    "dispatch.prx_start_module_unexpected_res",
                    format_args!(
                        "_sys_prx_start_module id={id:#010x} cmd=2 reported res={res:#018x} \
                         after phase 1 returned NO_ENTRY; returning res & 0xFFFF_FFFF \
                         (CELL_OK when the low word is zero)"
                    ),
                );
                Lv2Dispatch::immediate(res & 0xFFFF_FFFF)
            }
            other => {
                self.log_invariant_break(
                    "dispatch.prx_start_module_unknown_cmd",
                    format_args!(
                        "_sys_prx_start_module id={id:#010x} cmd nibble {other:#x} is not \
                         GET_ENTRY(1) or REPORT_RESULT(2); returning CELL_PRX_ERROR_ERROR"
                    ),
                );
                Lv2Dispatch::immediate(CELL_PRX_ERROR_ERROR.into())
            }
        }
    }

    /// `_sys_prx_stop_module` (482): the two-phase stop handshake.
    ///
    /// `pOpt->cmd & 0xF` selects the arm, as sc 481 does:
    ///
    /// 1. Phase 1 moves a `Started` module to `Stopping` and hands
    ///    the caller the entry to invoke. CellGov never runs guest
    ///    `module_stop`, so this phase reports `NO_ENTRY`. liblv2
    ///    then skips the call and reports zero.
    /// 2. Phase 2 with `res == 0` completes `Stopping -> Stopped`,
    ///    and a later unload withdraws the module.
    ///
    /// An unknown `id` together with a null `pOpt` answers
    /// `CELL_ESRCH`. The kernel's own precedence between the two is
    /// unestablished.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when `id` names no loaded module.
    /// - `CELL_EINVAL` for null `pOpt`.
    /// - `CELL_PRX_ERROR_NOT_STARTED` / `ALREADY_STOPPED` /
    ///   `ALREADY_STOPPING` when cmd 1 / 4 / 8 finds the module in
    ///   the wrong state.
    /// - `CELL_PRX_ERROR_CAN_NOT_STOP` when phase 2 reports `res == 1`.
    /// - `CELL_PRX_ERROR_ERROR` for a command nibble outside
    ///   1 / 2 / 4 / 8.
    /// - `CELL_EFAULT` when `pOpt` is unreadable or the struct would
    ///   not fit inside the 32-bit guest address space.
    pub(in crate::host::dispatch_route) fn dispatch_prx_stop_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use crate::prx_registry::PrxState;
        use cellgov_ps3_abi::sys_prx::{
            start_cmd, start_stop_option as opt, stop_cmd, CELL_PRX_ERROR_ALREADY_STOPPED,
            CELL_PRX_ERROR_ALREADY_STOPPING, CELL_PRX_ERROR_CAN_NOT_STOP, CELL_PRX_ERROR_ERROR,
            CELL_PRX_ERROR_NOT_STARTED,
        };

        let id = args[0] as u32;
        let p_opt = args[2] as u32;
        if self.state.prx_registry.lookup_by_id(id).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        if p_opt == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        // The base struct (through `res`) must fit below the 4 GiB
        // boundary; `entry2`'s reach is gated after `size` is known.
        if p_opt.checked_add(opt::MIN_SIZE as u32).is_none() {
            self.log_invariant_break(
                "dispatch.prx_stop_module_p_opt_wraps",
                format_args!(
                    "_sys_prx_stop_module option struct at p_opt={p_opt:#010x} wraps u32; \
                     returning CELL_EFAULT (struct does not fit in 32-bit guest address space)"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        let Some(size) = self.read_be_u64(rt, p_opt + opt::SIZE_OFFSET) else {
            self.log_invariant_break(
                "dispatch.prx_stop_module_size_unreadable",
                format_args!(
                    "_sys_prx_stop_module pOpt={p_opt:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        // Extended struct: entry2 at +0x20 must also fit.
        if size != opt::MIN_SIZE && p_opt.checked_add(opt::ENTRY2_OFFSET + 8).is_none() {
            self.log_invariant_break(
                "dispatch.prx_stop_module_entry2_wraps",
                format_args!(
                    "_sys_prx_stop_module extended option struct at p_opt={p_opt:#010x} \
                     (size={size:#x}) wraps u32 at entry2; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let Some(cmd) = self.read_be_u64(rt, p_opt + opt::CMD_OFFSET) else {
            self.log_invariant_break(
                "dispatch.prx_stop_module_cmd_unreadable",
                format_args!(
                    "_sys_prx_stop_module pOpt={p_opt:#010x} cmd field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };

        // Write NO_ENTRY to `entry` (and `entry2` for the extended
        // struct); shared by the cmd 1 and cmd 4 arms.
        let no_entry_effects = |host: &Self| {
            let mut effects =
                vec![host.write_be_u64(requester, p_opt + opt::ENTRY_OFFSET, opt::NO_ENTRY, tick)];
            if size != opt::MIN_SIZE {
                effects.push(host.write_be_u64(
                    requester,
                    p_opt + opt::ENTRY2_OFFSET,
                    opt::NO_ENTRY,
                    tick,
                ));
            }
            effects
        };
        let wrong_state = |state: PrxState| match state {
            PrxState::Initialized => Some(CELL_PRX_ERROR_NOT_STARTED),
            PrxState::Stopped => Some(CELL_PRX_ERROR_ALREADY_STOPPED),
            PrxState::Stopping => Some(CELL_PRX_ERROR_ALREADY_STOPPING),
            PrxState::Started => None,
        };

        match cmd & start_cmd::MASK {
            start_cmd::GET_ENTRY => {
                let old = self
                    .state
                    .prx_registry
                    .begin_stop(id)
                    .expect("id lookup succeeded above");
                if let Some(err) = wrong_state(old) {
                    return Lv2Dispatch::immediate(err.into());
                }
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: no_entry_effects(self),
                }
            }
            start_cmd::REPORT_RESULT => {
                let Some(res) = self.read_be_u64(rt, p_opt + opt::RES_OFFSET) else {
                    self.log_invariant_break(
                        "dispatch.prx_stop_module_res_unreadable",
                        format_args!(
                            "_sys_prx_stop_module pOpt={p_opt:#010x} res field unreadable; \
                             returning CELL_EFAULT"
                        ),
                    );
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                match res {
                    0 => {
                        if !self.state.prx_registry.finish_stop(id) {
                            // liblv2.sprx issues phase 2 only straight
                            // after an accepted phase 1, so no
                            // firmware caller produces this and the
                            // kernel's answer is unknown. CELL_OK
                            // without a transition leaves the registry
                            // consistent.
                            self.log_invariant_break(
                                "dispatch.prx_stop_module_unexpected_state",
                                format_args!(
                                    "_sys_prx_stop_module id={id:#010x} cmd=2 res=0 but the \
                                     module is not in the Stopping state; returning CELL_OK \
                                     without a transition"
                                ),
                            );
                        }
                        Lv2Dispatch::immediate(0)
                    }
                    1 => {
                        // Phase 1 handed back NO_ENTRY, so the guest
                        // called nothing; a can-not-stop report is a
                        // gap signal even though the code is faithful.
                        self.log_invariant_break(
                            "dispatch.prx_stop_module_unexpected_res",
                            format_args!(
                                "_sys_prx_stop_module id={id:#010x} cmd=2 reported res=1 after \
                                 phase 1 returned NO_ENTRY; returning \
                                 CELL_PRX_ERROR_CAN_NOT_STOP"
                            ),
                        );
                        Lv2Dispatch::immediate(CELL_PRX_ERROR_CAN_NOT_STOP.into())
                    }
                    other => {
                        // `res` carries the sign-extended s32 a
                        // module_stop returned, so any value can
                        // arrive here. Only 0 and 1 have a known
                        // meaning. CELL_OK for the rest is CellGov's
                        // own answer.
                        self.log_invariant_break(
                            "dispatch.prx_stop_module_unexpected_res",
                            format_args!(
                                "_sys_prx_stop_module id={id:#010x} cmd=2 reported \
                                 res={other:#018x} after phase 1 returned NO_ENTRY; returning \
                                 CELL_OK with no state change"
                            ),
                        );
                        Lv2Dispatch::immediate(0)
                    }
                }
            }
            stop_cmd::GET_ENTRIES | stop_cmd::REPORT_ENTRIES_RESULT => {
                let state = self
                    .state
                    .prx_registry
                    .lookup_by_id(id)
                    .expect("id lookup succeeded above")
                    .state();
                if let Some(err) = wrong_state(state) {
                    return Lv2Dispatch::immediate(err.into());
                }
                // The nibble selects this arm, but the branch below
                // tests the FULL cmd value: only exactly 4 hands back
                // the entries. 8, 0x14, 0x18, ... all fall through.
                // liblv2.sprx writes only 4 and 8.
                if cmd == stop_cmd::GET_ENTRIES {
                    return Lv2Dispatch::Immediate {
                        code: 0,
                        effects: no_entry_effects(self),
                    };
                }
                self.log_invariant_break(
                    "dispatch.prx_stop_module_teardown_report_unmodelled",
                    format_args!(
                        "_sys_prx_stop_module id={id:#010x} cmd={cmd:#x} is not exactly \
                         GET_ENTRIES(4); cmd 8 is the teardown handshake's report phase \
                         and any other nibble-4/8 value has no known meaning. Returning \
                         CELL_OK without reading res or completing Stopping -> Stopped"
                    ),
                );
                Lv2Dispatch::immediate(0)
            }
            other => {
                self.log_invariant_break(
                    "dispatch.prx_stop_module_unknown_cmd",
                    format_args!(
                        "_sys_prx_stop_module id={id:#010x} cmd nibble {other:#x} is not 1, 2, \
                         4, or 8; returning CELL_PRX_ERROR_ERROR"
                    ),
                );
                Lv2Dispatch::immediate(CELL_PRX_ERROR_ERROR.into())
            }
        }
    }

    /// `_sys_prx_unload_module` (483): withdraw an `Initialized` or
    /// `Stopped` module, refuse a `Started` / `Stopping` one.
    ///
    /// Boot-loaded firmware modules are started, since their images
    /// back the GOT slots the title calls through. They refuse until
    /// the guest completes the sc 482 stop handshake. An sc 480 miss
    /// stub the guest never started withdraws with `CELL_OK` and frees
    /// its id.
    ///
    /// # Errors
    ///
    /// - `CELL_PRX_ERROR_UNKNOWN_MODULE` when `id` names no loaded module.
    /// - `CELL_PRX_ERROR_NOT_REMOVABLE` for a started or stopping
    ///   resident module.
    pub(in crate::host::dispatch_route) fn dispatch_prx_unload_module(
        &mut self,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        use crate::prx_registry::PrxState;
        use cellgov_ps3_abi::sys_prx::{
            CELL_PRX_ERROR_NOT_REMOVABLE, CELL_PRX_ERROR_UNKNOWN_MODULE,
        };

        let id = args[0] as u32;
        match self.state.prx_registry.lookup_by_id(id) {
            None => Lv2Dispatch::immediate(CELL_PRX_ERROR_UNKNOWN_MODULE.into()),
            Some(entry) => match entry.state() {
                PrxState::Initialized | PrxState::Stopped => {
                    let removed = self.state.prx_registry.withdraw_removable(id);
                    debug_assert!(removed.is_some(), "lookup said present and removable");
                    Lv2Dispatch::immediate(0)
                }
                PrxState::Started | PrxState::Stopping => {
                    let stem = entry.stem().to_string();
                    self.obs.prx_unload_rejections += 1;
                    self.log_invariant_break(
                        "dispatch.prx_unload_module_resident",
                        format_args!(
                            "_sys_prx_unload_module id={id:#010x} ({stem}) is a started \
                             resident module; returning CELL_PRX_ERROR_NOT_REMOVABLE"
                        ),
                    );
                    Lv2Dispatch::immediate(CELL_PRX_ERROR_NOT_REMOVABLE.into())
                }
            },
        }
    }

    /// Read a big-endian `u64` from committed guest memory.
    fn read_be_u64(&self, rt: &dyn Lv2Runtime, addr: u32) -> Option<u64> {
        let bytes = rt.read_committed(u64::from(addr), 8)?;
        Some(u64::from_be_bytes(bytes[..8].try_into().ok()?))
    }

    /// Stage a big-endian `u64` write to guest memory.
    fn write_be_u64(&self, requester: UnitId, addr: u32, value: u64, tick: GuestTicks) -> Effect {
        Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(addr, 8),
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        }
    }

    /// `_sys_prx_register_module` (484): returns
    /// CELL_PRX_ERROR_ELF_IS_REGISTERED for non-VSH callers.
    pub(in crate::host::dispatch_route) fn dispatch_prx_register_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_ELF_IS_REGISTERED;

        let opt = args[1];
        if opt == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let Some(size) = read_be_u64(rt, opt) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        // 0x1c / 0x20 are the legacy option forms, which carry no
        // type word; treating them as type = 0 skips the branch
        // entirely, so they need no field reads here.
        let (module_type, stub_ea, stub_size) = match size {
            0x1c | 0x20 => (0u64, 0u32, 0u32),
            0x30 => {
                let Some(t) = read_be_u64(rt, opt + 0x08) else {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                let (Some(ea), Some(sz)) =
                    (read_be_u32(rt, opt + 0x20), read_be_u32(rt, opt + 0x24))
                else {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                (t, ea, sz)
            }
            _ => return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
        };
        self.obs.prx_register_module_count += 1;

        if module_type & 0x1 == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if !self.is_coreos() {
            // Only a CoreOS process may hand the kernel its own tables.
            return Lv2Dispatch::immediate(CELL_PRX_ERROR_ELF_IS_REGISTERED.into());
        }
        self.obs.prx_register_module_manual_count += 1;
        let effects = self.link_manual_imports(stub_ea, stub_size, requester, rt, tick);
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// Bind the import table at `[stub_ea, stub_ea + stub_size)` against
    /// the firmware export map, returning one `SharedWriteIntent` per
    /// resolved GOT slot.
    ///
    /// The caller-supplied table is guest data and may be uninitialised
    /// -- a CoreOS module can reach this arm with a garbage pointer and
    /// a huge size. Every read is bounds-checked through the runtime and
    /// a failed read ends the walk rather than faulting the host; an
    /// unresolved NID is left alone so the guest's own stub address
    /// stays in the slot. Each way the walk can stop early names its
    /// own invariant break.
    fn link_manual_imports(
        &mut self,
        stub_ea: u32,
        stub_size: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Vec<Effect> {
        use cellgov_ps3_abi::elf::{
            PRX_IMPORT_ENTRY_MIN_SIZE, PRX_IMPORT_NAME_PTR_OFFSET, PRX_IMPORT_NIDS_PTR_OFFSET,
            PRX_IMPORT_NUM_FUNC_OFFSET, PRX_IMPORT_SIZE_OFFSET, PRX_IMPORT_STUB_PTR_OFFSET,
            PRX_NAME_MAX_LEN,
        };

        let mut effects = Vec::new();
        let Some(table_end) = stub_ea.checked_add(stub_size) else {
            self.log_invariant_break(
                "dispatch.prx_register_module_table_wraps",
                format_args!(
                    "import table [0x{stub_ea:08x}, +0x{stub_size:x}) wraps u32; \
                     nothing linked"
                ),
            );
            return effects;
        };
        let mut cursor = stub_ea;
        while cursor < table_end {
            let Some(hdr) =
                rt.read_committed(u64::from(cursor), PRX_IMPORT_ENTRY_MIN_SIZE as usize)
            else {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_unreadable",
                    format_args!(
                        "import entry header at 0x{cursor:08x} is unreadable; the walk \
                         stops short of the table end 0x{table_end:08x}"
                    ),
                );
                break;
            };
            let entry_size = hdr[PRX_IMPORT_SIZE_OFFSET];
            if entry_size < PRX_IMPORT_ENTRY_MIN_SIZE {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_size_invalid",
                    format_args!(
                        "import entry at 0x{cursor:08x} declares size {entry_size} below the \
                         {PRX_IMPORT_ENTRY_MIN_SIZE}-byte minimum; the walk stops short of \
                         the table end 0x{table_end:08x}"
                    ),
                );
                break;
            }
            let func_count = u16::from_be_bytes([
                hdr[PRX_IMPORT_NUM_FUNC_OFFSET],
                hdr[PRX_IMPORT_NUM_FUNC_OFFSET + 1],
            ]);
            let nids_ptr = u32::from_be_bytes([
                hdr[PRX_IMPORT_NIDS_PTR_OFFSET],
                hdr[PRX_IMPORT_NIDS_PTR_OFFSET + 1],
                hdr[PRX_IMPORT_NIDS_PTR_OFFSET + 2],
                hdr[PRX_IMPORT_NIDS_PTR_OFFSET + 3],
            ]);
            let stub_ptr = u32::from_be_bytes([
                hdr[PRX_IMPORT_STUB_PTR_OFFSET],
                hdr[PRX_IMPORT_STUB_PTR_OFFSET + 1],
                hdr[PRX_IMPORT_STUB_PTR_OFFSET + 2],
                hdr[PRX_IMPORT_STUB_PTR_OFFSET + 3],
            ]);
            let name_ptr = u32::from_be_bytes([
                hdr[PRX_IMPORT_NAME_PTR_OFFSET],
                hdr[PRX_IMPORT_NAME_PTR_OFFSET + 1],
                hdr[PRX_IMPORT_NAME_PTR_OFFSET + 2],
                hdr[PRX_IMPORT_NAME_PTR_OFFSET + 3],
            ]);
            // The cap and the lossy decode mirror the loader-side
            // decoders that mint the `firmware_exports` keys
            // (`cellgov_ppu::prx::read_cstring`,
            // `cellgov_ppu::sprx::parse::read_cstring`): a name the
            // loader accepted must produce the identical lookup key
            // here, or the library silently never resolves.
            let library = match rt.read_committed_until(u64::from(name_ptr), PRX_NAME_MAX_LEN, 0) {
                Some(bytes) => self
                    .derived
                    .firmware_exports
                    .get(String::from_utf8_lossy(bytes).as_ref()),
                None => {
                    // An unreadable name resolves under no library:
                    // the entry's NIDs still walk (so the witness
                    // counts them) but every lookup misses.
                    self.log_invariant_break(
                        "dispatch.prx_register_module_name_unreadable",
                        format_args!(
                            "import entry at 0x{cursor:08x}: library-name pointer \
                             0x{name_ptr:08x} is unreadable or unterminated within \
                             {PRX_NAME_MAX_LEN} bytes; its {func_count} import NID(s) \
                             stay unresolved",
                        ),
                    );
                    None
                }
            };
            for i in 0..u32::from(func_count) {
                let (Some(nid_at), Some(slot_at)) = (
                    nids_ptr.checked_add(i * 4).map(u64::from),
                    stub_ptr.checked_add(i * 4).map(u64::from),
                ) else {
                    self.log_invariant_break(
                        "dispatch.prx_register_module_nid_table_truncated",
                        format_args!(
                            "import entry at 0x{cursor:08x}: NID slot {i} of {func_count} \
                             wraps u32 (nids=0x{nids_ptr:08x} stubs=0x{stub_ptr:08x}); the \
                             remaining NIDs stay unresolved"
                        ),
                    );
                    break;
                };
                let Some(nid) = read_be_u32(rt, nid_at) else {
                    self.log_invariant_break(
                        "dispatch.prx_register_module_nid_table_truncated",
                        format_args!(
                            "import entry at 0x{cursor:08x}: NID {i} of {func_count} at \
                             0x{nid_at:08x} is unreadable; the remaining NIDs stay unresolved"
                        ),
                    );
                    break;
                };
                let Some(&opd) = library.and_then(|lib| lib.get(&nid)) else {
                    self.obs.prx_register_module_unresolved += 1;
                    continue;
                };
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(slot_at as u32, 4),
                    bytes: WritePayload::from_slice(&opd.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                });
                self.obs.prx_register_module_linked += 1;
            }
            let Some(next) = cursor.checked_add(u32::from(entry_size)) else {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_advance_wraps",
                    format_args!(
                        "import entry at 0x{cursor:08x} plus its size {entry_size} wraps u32; \
                         the walk stops short of the table end 0x{table_end:08x}"
                    ),
                );
                break;
            };
            cursor = next;
        }
        effects
    }

    /// `_sys_prx_register_library` (486): gates the library
    /// descriptor, then returns CELL_OK (the kernel's no-match
    /// success path).
    ///
    /// Associating the descriptor with a loaded module's export table
    /// is not modelled; CellGov publishes every firmware module's
    /// exports at boot, so a caller-registered library adds no
    /// resolvable symbol.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when `library` is null or unmapped; the address
    ///   is checked before the descriptor is touched.
    pub(in crate::host::dispatch_route) fn dispatch_prx_register_library(
        &self,
        args: [u64; 8],
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let library = args[0];
        if library == 0 || rt.read_committed(library, 1).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// `_sys_prx_get_module_list` (494): fills `pInfo->idlist` and
    /// writes `pInfo->count`, filtering liblv2.sprx.
    ///
    /// Struct layout, as liblv2.sprx's `sys_prx_get_module_list`
    /// builds it on its own stack: `size@0` (u64, declared `0x20`),
    /// `pad@8` (never written), `max@0xC`, `count@0x10`,
    /// `idlist@0x14`, `unk@0x18`, tail padding to `0x20`. Only
    /// `[p_info, p_info+0x18)` is touched. liblv2.sprx always passes
    /// `flags = 2`; `flags & 0x2 == 0` short-circuits to CELL_OK.
    /// CELL_EFAULT on null `pInfo`.
    ///
    /// `size` selects the layout. A value other than `0x20` names a
    /// struct whose `max` / `count` / `idlist` sit elsewhere, and that
    /// layout is not modelled: the call fills nothing, answers
    /// CELL_OK, and logs a break.
    ///
    /// # Cross-module contract
    ///
    /// Slot writes and the trailing count write are co-emitted in one
    /// `Lv2Dispatch::Immediate` batch so `apply_lv2_effects` can
    /// commit them all-or-none.
    pub(in crate::host::dispatch_route) fn dispatch_prx_get_module_list(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let modelled_size = cellgov_ps3_abi::sys_prx::get_module_list_option::SIZE;

        let flags = args[0];
        let p_info = args[1] as u32;
        if flags & 0x2 == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if p_info == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        if p_info.checked_add(0x18).is_none() {
            self.log_invariant_break(
                "dispatch.prx_module_list_p_info_wraps",
                format_args!(
                    "sys_prx_get_module_list pInfo struct [p_info, p_info+0x18) wraps u32: \
                     pInfo={p_info:#010x}; returning CELL_EFAULT (struct does not fit in \
                     32-bit guest address space)"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let Some(declared_size) = self.read_be_u64(rt, p_info) else {
            self.log_invariant_break(
                "dispatch.prx_module_list_unreadable_pinfo",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        if declared_size != modelled_size {
            self.log_invariant_break(
                "dispatch.prx_module_list_unknown_struct_size",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} declares \
                     size={declared_size:#x}, not {modelled_size:#x}; the other \
                     layout is not modelled, so nothing is filled in and CELL_OK is \
                     returned"
                ),
            );
            return Lv2Dispatch::immediate(0);
        }
        let mut effects = Vec::new();
        let max_addr = p_info.wrapping_add(0x0C);
        let count_addr = p_info.wrapping_add(0x10);
        let idlist_ptr_addr = p_info.wrapping_add(0x14);
        let Some(max_bytes) = rt.read_committed(u64::from(max_addr), 4) else {
            self.log_invariant_break(
                "dispatch.prx_module_list_unreadable_pinfo",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} max field at \
                     {max_addr:#010x} unreadable; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let max = u32::from_be_bytes([max_bytes[0], max_bytes[1], max_bytes[2], max_bytes[3]]);
        let Some(idlist_bytes) = rt.read_committed(u64::from(idlist_ptr_addr), 4) else {
            self.log_invariant_break(
                "dispatch.prx_module_list_unreadable_pinfo",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} idlist field at \
                     {idlist_ptr_addr:#010x} unreadable; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let idlist_ptr = u32::from_be_bytes([
            idlist_bytes[0],
            idlist_bytes[1],
            idlist_bytes[2],
            idlist_bytes[3],
        ]);
        let liblv2_id = self
            .state
            .prx_registry
            .lookup_by_path("liblv2.sprx")
            .map(|e| e.kernel_id());
        let mut count: u32 = 0;
        if idlist_ptr != 0 {
            for kid in self.state.prx_registry.ids() {
                if Some(kid) == liblv2_id {
                    continue;
                }
                if count >= max {
                    break;
                }
                debug_assert!(
                    count
                        .checked_mul(4)
                        .and_then(|off| idlist_ptr.checked_add(off))
                        .and_then(|s| s.checked_add(4))
                        .is_some(),
                    "sys_prx_get_module_list 4-byte slot write at idlist_ptr+count*4 \
                     wraps u32: idlist_ptr={idlist_ptr:#010x} count={count}",
                );
                let slot = idlist_ptr.wrapping_add(count.wrapping_mul(4));
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(slot, 4),
                    bytes: WritePayload::from_slice(&kid.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                });
                count += 1;
            }
        }
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(count_addr, 4),
            bytes: WritePayload::from_slice(&count.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        });
        Lv2Dispatch::Immediate { code: 0, effects }
    }
}
