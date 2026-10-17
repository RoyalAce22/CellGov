//! The `_sys_prx_*` start, stop and unload arms.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use cellgov_ps3_abi::lv2::prx::return_code;

use crate::host::guest_struct::read_be_u64;
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// `_sys_prx_start_module` (481): the two-phase start handshake.
    ///
    /// `pOpt->cmd & 0xF` selects the phase:
    ///
    /// 1. Phase 1 moves an `Initialized` module to `Starting` and hands
    ///    the caller the entry to invoke. CellGov runs every firmware
    ///    module's `module_start` itself at boot, so this phase reports
    ///    `NO_ENTRY`. A module in any other state answers
    ///    `CELL_PRX_ERROR_ERROR`. No capture repeats phase 1 on a
    ///    `Starting` module; CellGov refuses it as it does a started one.
    /// 2. Phase 2 reports what that entry returned. A report of
    ///    `SYS_PRX_RESIDENT` marks the module started, and a later
    ///    unload then answers `NOT_REMOVABLE`. Any other report
    ///    withdraws the module.
    ///
    /// The `lv2_prx_states` capture reads the phase from the low
    /// nibble: 0x11 and 0x21 answer as 1, and 3, 0x10 and 0 answer
    /// `CELL_PRX_ERROR_ERROR` with nothing written.
    ///
    /// `pOpt->size` is never validated. The arm compares it against
    /// `MIN_SIZE` only to decide whether `entry2` exists. liblv2.sprx
    /// always declares the extended `0x28` form, so a firmware caller
    /// always has both slots.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for null `id`, an `id` with bit 31 set, or null
    ///   `pOpt`. The `lv2_prx_start_gate` capture answers EINVAL for
    ///   such an id with a readable option, before the lookup.
    /// - `CELL_ESRCH` when `id` names no loaded module.
    /// - `CELL_PRX_ERROR_ERROR` for a command nibble that is neither
    ///   1 nor 2, or a phase 1 to a module not just loaded.
    /// - `CELL_EFAULT` when `pOpt` is unreadable or the struct would
    ///   not fit inside the 32-bit guest address space.
    /// - `CELL_EINVAL` when `id` or `pOpt` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`].
    /// - The low word of `pOpt->res` when a phase-2 report carries a
    ///   value other than `SYS_PRX_RESIDENT`, as the console answers 1
    ///   and 5. A zero low word answers CELL_OK.
    pub(in crate::host::dispatch_route) fn dispatch_prx_start_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::prx::{
            start_cmd, start_stop_option as opt, CELL_PRX_ERROR_ERROR, PRX_ID_SIGN_BIT,
            SYS_PRX_RESIDENT,
        };
        use cellgov_ps3_abi::lv2::syscall;

        let Some([id, p_opt]) = self.narrow_u32_args(
            syscall::SYS_PRX_START_MODULE,
            [("id", args[0]), ("pOpt", args[2])],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if id == 0 || id & PRX_ID_SIGN_BIT != 0 || p_opt == 0 {
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
                // Only a module just loaded starts: the console refuses a
                // second start of a started module and a start of a
                // stopped one (`lv2_prx_states`, `lv2_prx_states_more`).
                if self.state.prx_registry.begin_start(id)
                    != Some(crate::prx_registry::PrxState::Initialized)
                {
                    return Lv2Dispatch::immediate(return_code(CELL_PRX_ERROR_ERROR));
                }
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
                // returned, so its low 32 bits are that code. The
                // console answers a report of 1 or 5 with that value
                // and withdraws the module (`lv2_prx_states_more`,
                // `lv2_prx_states_last`).
                self.log_invariant_break(
                    "dispatch.prx_start_module_unexpected_res",
                    format_args!(
                        "_sys_prx_start_module id={id:#010x} cmd=2 reported res={res:#018x} \
                         after phase 1 returned NO_ENTRY; withdrawing the module and returning \
                         res & 0xFFFF_FFFF (CELL_OK when the low word is zero)"
                    ),
                );
                self.state.prx_registry.withdraw(id);
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
                Lv2Dispatch::immediate(return_code(CELL_PRX_ERROR_ERROR))
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
    /// 2. Phase 2 with `res == 0` completes `Stopping -> Stopped`, or
    ///    stops a module that never started, and a later unload
    ///    withdraws the module. `res == 1` is
    ///    `CELL_PRX_ERROR_CAN_NOT_STOP` and puts a stopping module
    ///    back to started; any other value answers CELL_OK with no
    ///    transition.
    ///
    /// Nibbles 4 and 8 are the pair the teardown helper runs: 4 hands
    /// back the entries, 8 reports what they returned. Neither moves
    /// the module's state.
    ///
    /// A null `pOpt` answers `CELL_EINVAL` before the id is looked up,
    /// as the `lv2_prx_gates` capture reads for an unknown id. Unlike
    /// sc 481, an id with bit 31 set is only looked up.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for null `pOpt`.
    /// - `CELL_ESRCH` when `id` names no loaded module.
    /// - `CELL_PRX_ERROR_NOT_STARTED` / `ALREADY_STOPPED` /
    ///   `ALREADY_STOPPING` when cmd 1 / 4 / 8 finds the module in
    ///   the wrong state. Nibble 4 answers a stopped module
    ///   `ALREADY_STOPPING`.
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
        if p_opt == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if self.state.prx_registry.lookup_by_id(id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
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
        // No capture stops a module between its two start phases;
        // CellGov answers it as one that never started.
        let wrong_state = |state: PrxState| match state {
            PrxState::Initialized | PrxState::Starting => Some(CELL_PRX_ERROR_NOT_STARTED),
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
                    return Lv2Dispatch::immediate(return_code(err));
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
                        // A report to a module that never started
                        // stops it: the console answers CELL_OK and
                        // then ALREADY_STOPPED (`lv2_prx_states_more`).
                        if !self.state.prx_registry.finish_stop(id)
                            && !self.state.prx_registry.stop_unstarted(id)
                        {
                            // liblv2.sprx issues phase 2 only straight
                            // after an accepted phase 1, so no
                            // firmware caller produces this and the
                            // kernel's answer for a started or stopped
                            // module is unknown. CELL_OK without a
                            // transition leaves the registry
                            // consistent.
                            self.log_invariant_break(
                                "dispatch.prx_stop_module_unexpected_state",
                                format_args!(
                                    "_sys_prx_stop_module id={id:#010x} cmd=2 res=0 but the \
                                     module is neither Stopping nor Initialized; returning \
                                     CELL_OK without a transition"
                                ),
                            );
                        }
                        Lv2Dispatch::immediate(0)
                    }
                    1 => {
                        // Phase 1 handed back NO_ENTRY, so the guest
                        // called nothing; a can-not-stop report is a
                        // gap signal even though the code is faithful.
                        // The console puts a stopping module back to
                        // started (`lv2_prx_states_more`).
                        self.log_invariant_break(
                            "dispatch.prx_stop_module_unexpected_res",
                            format_args!(
                                "_sys_prx_stop_module id={id:#010x} cmd=2 reported res=1 after \
                                 phase 1 returned NO_ENTRY; returning \
                                 CELL_PRX_ERROR_CAN_NOT_STOP"
                            ),
                        );
                        self.state.prx_registry.cancel_stop(id);
                        Lv2Dispatch::immediate(return_code(CELL_PRX_ERROR_CAN_NOT_STOP))
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
            stop_cmd::GET_ENTRIES => {
                let state = self
                    .state
                    .prx_registry
                    .lookup_by_id(id)
                    .expect("id lookup succeeded above")
                    .state();
                // The console answers a stopped module ALREADY_STOPPING
                // here, not ALREADY_STOPPED (`lv2_prx_states_more`).
                if state == PrxState::Stopped {
                    return Lv2Dispatch::immediate(return_code(CELL_PRX_ERROR_ALREADY_STOPPING));
                }
                if let Some(err) = wrong_state(state) {
                    return Lv2Dispatch::immediate(return_code(err));
                }
                // The nibble selects the arm: the console answers 0x14
                // as it answers 4 (`lv2_prx_states`).
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: no_entry_effects(self),
                }
            }
            stop_cmd::REPORT_ENTRIES_RESULT => {
                let state = self
                    .state
                    .prx_registry
                    .lookup_by_id(id)
                    .expect("id lookup succeeded above")
                    .state();
                if let Some(err) = wrong_state(state) {
                    return Lv2Dispatch::immediate(return_code(err));
                }
                // The console answers cmd 8 to a started module with
                // CELL_OK and writes neither entry (`lv2_prx_states`).
                self.log_invariant_break(
                    "dispatch.prx_stop_module_teardown_report_unmodelled",
                    format_args!(
                        "_sys_prx_stop_module id={id:#010x} cmd={cmd:#x} is the teardown \
                         handshake's report phase. Returning CELL_OK without reading res or \
                         completing Stopping -> Stopped"
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
                Lv2Dispatch::immediate(return_code(CELL_PRX_ERROR_ERROR))
            }
        }
    }

    /// `_sys_prx_unload_module` (483): withdraw an `Initialized` or
    /// `Stopped` module with `CELL_OK`. For a `Started`, `Starting` or
    /// `Stopping` module the call answers `NOT_REMOVABLE` and still
    /// withdraws it, as the `lv2_prx_states_last` capture shows for a
    /// started module and for one between the two start phases. No
    /// capture unloads a `Stopping` module; CellGov withdraws it as it
    /// does a started one.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when `id` names no loaded module, as the
    ///   `lv2_prx_states` capture answers an id already unloaded.
    /// - `CELL_PRX_ERROR_NOT_REMOVABLE` for a module in the `Started`,
    ///   `Starting` or `Stopping` state, which the call withdraws.
    /// - `CELL_EINVAL` when `id` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`].
    pub(in crate::host::dispatch_route) fn dispatch_prx_unload_module(
        &mut self,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        use crate::prx_registry::PrxState;
        use cellgov_ps3_abi::lv2::prx::CELL_PRX_ERROR_NOT_REMOVABLE;
        use cellgov_ps3_abi::lv2::syscall;

        let Some([id]) = self.narrow_u32_args(syscall::SYS_PRX_UNLOAD_MODULE, [("id", args[0])])
        else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        match self.state.prx_registry.lookup_by_id(id) {
            None => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            Some(entry) => match entry.state() {
                PrxState::Initialized | PrxState::Stopped => {
                    let removed = self.state.prx_registry.withdraw_removable(id);
                    debug_assert!(removed.is_some(), "lookup said present and removable");
                    Lv2Dispatch::immediate(0)
                }
                PrxState::Started | PrxState::Starting | PrxState::Stopping => {
                    // The refusal still withdraws the module: every later
                    // call on the id answers ESRCH (`lv2_prx_states_last`).
                    self.obs.prx_unload_rejections += 1;
                    self.state.prx_registry.withdraw(id);
                    Lv2Dispatch::immediate(return_code(CELL_PRX_ERROR_NOT_REMOVABLE))
                }
            },
        }
    }
}
