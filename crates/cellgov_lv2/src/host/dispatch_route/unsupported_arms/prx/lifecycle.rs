//! The `_sys_prx_*` start, stop and unload arms.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::guest_struct::read_be_u64;
use crate::host::{Lv2Host, Lv2Runtime};

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
    /// - `CELL_EINVAL` when `id` or `pOpt` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`].
    /// - The low word of `pOpt->res` when a phase-2 report carries a
    ///   value other than `SYS_PRX_RESIDENT`. A zero low word answers
    ///   CELL_OK.
    pub(in crate::host::dispatch_route) fn dispatch_prx_start_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::prx::{
            start_cmd, start_stop_option as opt, CELL_PRX_ERROR_ERROR, SYS_PRX_RESIDENT,
        };
        use cellgov_ps3_abi::lv2::syscall;

        let Some([id, p_opt]) = self.narrow_u32_args(
            syscall::SYS_PRX_START_MODULE,
            [("id", args[0]), ("pOpt", args[2])],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if id == 0 || p_opt == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if self.state.prx_registry.lookup_by_id(id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
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
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }

        let Some(size) = read_be_u64(rt, u64::from(p_opt + opt::SIZE_OFFSET)) else {
            self.log_invariant_break(
                "dispatch.prx_start_module_size_unreadable",
                format_args!(
                    "_sys_prx_start_module pOpt={p_opt:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let Some(cmd) = read_be_u64(rt, u64::from(p_opt + opt::CMD_OFFSET)) else {
            self.log_invariant_break(
                "dispatch.prx_start_module_cmd_unreadable",
                format_args!(
                    "_sys_prx_start_module pOpt={p_opt:#010x} cmd field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
                let Some(res) = read_be_u64(rt, u64::from(p_opt + opt::RES_OFFSET)) else {
                    self.log_invariant_break(
                        "dispatch.prx_start_module_res_unreadable",
                        format_args!(
                            "_sys_prx_start_module pOpt={p_opt:#010x} res field unreadable; \
                             returning CELL_EFAULT"
                        ),
                    );
                    return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
    ///    and a later unload withdraws the module. `res == 1` is
    ///    `CELL_PRX_ERROR_CAN_NOT_STOP`; any other value answers
    ///    CELL_OK with no transition.
    ///
    /// cmd 4 and cmd 8 are the pair the teardown helper runs: 4 hands
    /// back the entries, 8 reports what they returned. Neither moves
    /// the module's state.
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
    /// - `CELL_EINVAL` when `id` or `pOpt` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`].
    pub(in crate::host::dispatch_route) fn dispatch_prx_stop_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use crate::prx_registry::PrxState;
        use cellgov_ps3_abi::lv2::prx::{
            start_cmd, start_stop_option as opt, stop_cmd, CELL_PRX_ERROR_ALREADY_STOPPED,
            CELL_PRX_ERROR_ALREADY_STOPPING, CELL_PRX_ERROR_CAN_NOT_STOP, CELL_PRX_ERROR_ERROR,
            CELL_PRX_ERROR_NOT_STARTED,
        };
        use cellgov_ps3_abi::lv2::syscall;

        let Some([id, p_opt]) = self.narrow_u32_args(
            syscall::SYS_PRX_STOP_MODULE,
            [("id", args[0]), ("pOpt", args[2])],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if self.state.prx_registry.lookup_by_id(id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if p_opt == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
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
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }

        let Some(size) = read_be_u64(rt, u64::from(p_opt + opt::SIZE_OFFSET)) else {
            self.log_invariant_break(
                "dispatch.prx_stop_module_size_unreadable",
                format_args!(
                    "_sys_prx_stop_module pOpt={p_opt:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let Some(cmd) = read_be_u64(rt, u64::from(p_opt + opt::CMD_OFFSET)) else {
            self.log_invariant_break(
                "dispatch.prx_stop_module_cmd_unreadable",
                format_args!(
                    "_sys_prx_stop_module pOpt={p_opt:#010x} cmd field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
                let Some(res) = read_be_u64(rt, u64::from(p_opt + opt::RES_OFFSET)) else {
                    self.log_invariant_break(
                        "dispatch.prx_stop_module_res_unreadable",
                        format_args!(
                            "_sys_prx_stop_module pOpt={p_opt:#010x} res field unreadable; \
                             returning CELL_EFAULT"
                        ),
                    );
                    return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
    /// - `CELL_EINVAL` when `id` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`].
    pub(in crate::host::dispatch_route) fn dispatch_prx_unload_module(
        &mut self,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        use crate::prx_registry::PrxState;
        use cellgov_ps3_abi::lv2::prx::{
            CELL_PRX_ERROR_NOT_REMOVABLE, CELL_PRX_ERROR_UNKNOWN_MODULE,
        };
        use cellgov_ps3_abi::lv2::syscall;

        let Some([id]) = self.narrow_u32_args(syscall::SYS_PRX_UNLOAD_MODULE, [("id", args[0])])
        else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
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
}
