//! Raw SPU problem-state windows: the 1 MiB of the creator's address
//! space at which a Raw SPU's local store and then its problem-state
//! registers appear, reached by plain PPU loads and stores.
//!
//! A window is a `ReadWrite` region. The Raw SPU create carves it out
//! of the reserved SPU range, and the Raw SPU destroy returns it. Its
//! bytes are a projection: the SPU's state lives in its unit, and the
//! runtime keeps the window in step with it.
//!
//! - After a batch commits, each write it made into a window reaches
//!   the unit: a local-store write lands in the local store, and a
//!   whole-word write of `SPU_In_Mbox`, `SPU_RunCntl`, `SPU_NPC` or a
//!   signal-notification register performs that register's operation.
//!   A load of `SPU_Out_Mbox` takes the message it read.
//! - At the end of every commit, the runtime rewrites the window's
//!   `SPU_Status`, `SPU_NPC`, `SPU_Mbox_Stat` and `SPU_Out_Mbox` words
//!   from the unit. It also rewrites the local-store pages that changed
//!   after a step of the Raw SPU itself or a DMA completion.
//!
//! A PPU block ends at the instruction that reaches a window, so the
//! next instruction sees the projection the commit made.
//!
//! [CBEA p:37 s:3.2] a local store can be aliased into the main storage domain.
//! [CBEA p:92 s:8.5.1] SPU_RunCntl 01 is a run request and 00 a stop request.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ProblemStateError, RestartError, SignalNotifier, StopRegisters};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::hw::spu::{
    SPU_IN_MBOX_DEPTH, SPU_IN_MBOX_OFFSET, SPU_LS_SIZE, SPU_MBOX_STAT_OFFSET, SPU_NPC_OFFSET,
    SPU_OUT_MBOX_OFFSET, SPU_RUN_CNTL_OFFSET, SPU_SIG_NOTIFY_1_OFFSET, SPU_SIG_NOTIFY_2_OFFSET,
    SPU_STATUS_C, SPU_STATUS_I, SPU_STATUS_OFFSET, SPU_STATUS_P,
};
use cellgov_ps3_abi::lv2::spu::{raw_spu, raw_window};
use cellgov_sync::MailboxId;
use cellgov_trace::HostWriter;

use super::spaces::AddressSpaceId;
use super::Runtime;

/// The label a window's region carries.
const WINDOW_LABEL: &str = "raw_spu_window";

/// The local-store bytes one projection compares and rewrites at once.
const LS_PAGE: u32 = 0x1000;

/// One mapped window.
#[derive(Debug, Clone, Copy)]
struct Window {
    id: u32,
    unit: UnitId,
    space: AddressSpaceId,
    base: u64,
}

impl Window {
    /// The address of the problem-state register at `offset`.
    fn register(&self, offset: u32) -> u64 {
        self.base + raw_window::PROBLEM_STATE + u64::from(offset)
    }

    /// The offset of `range` from the window's base, or `None` for a
    /// range that starts outside the window. An address past the window
    /// names no register and no local-store byte of this Raw SPU.
    fn offset_of(&self, range: ByteRange) -> Option<u64> {
        let offset = range.start().raw().checked_sub(self.base)?;
        (offset < raw_window::STRIDE).then_some(offset)
    }
}

/// The first address of Raw SPU `id`'s window.
fn window_base(id: u32) -> u64 {
    raw_window::BASE + u64::from(id) * raw_window::STRIDE
}

impl Runtime {
    /// The windows of the Raw SPUs that exist.
    fn raw_spu_windows(&self) -> Vec<Window> {
        self.lv2_host
            .raw_spu_units()
            .map(|(id, unit)| Window {
                id,
                unit,
                space: self.spaces.space_of(unit),
                base: window_base(id),
            })
            .collect()
    }

    /// Give the new Raw SPU `id`, run by `unit`, its window in `space`
    /// and an interrupt mailbox the PU interrupt mailbox register
    /// empties. The create's commit projects its stopped state into the
    /// window.
    pub(super) fn open_raw_spu_window(&mut self, id: u32, unit: UnitId, space: AddressSpaceId) {
        if let Err(err) = self.assign_unit_space(unit, space) {
            self.lv2_host.log_invariant_break(
                "runtime.raw_spu_space_unknown",
                format_args!("Raw SPU {id} ({unit:?}) cannot join space {space:?}: {err}"),
            );
        }
        let connected = self
            .registry
            .get_mut(unit)
            .map(|spu| spu.connect_out_intr_mbox());
        if !matches!(connected, Some(Ok(()))) {
            self.lv2_host.log_invariant_break(
                "runtime.raw_spu_intr_mbox_unconnected",
                format_args!("Raw SPU {id} ({unit:?}) has no interrupt mailbox: {connected:?}"),
            );
        }
        let (mem, _) = super::spaces::resolve_commit_targets(
            &mut self.memory,
            &mut self.reservations,
            &mut self.spaces,
            space,
        );
        if let Err(err) = mem.map_window(window_base(id), raw_window::STRIDE as usize, WINDOW_LABEL)
        {
            self.lv2_host.log_invariant_break(
                "runtime.raw_spu_window_unmapped",
                format_args!(
                    "Raw SPU {id}'s window at 0x{:x} found no reserved range: {err}; \
                     its loads and stores fault",
                    window_base(id)
                ),
            );
        }
    }

    /// Return the window of the destroyed Raw SPU `id`, run by `unit`,
    /// to the reserved range.
    pub(super) fn close_raw_spu_window(&mut self, id: u32, unit: UnitId) {
        let space = self.spaces.space_of(unit);
        let (mem, _) = super::spaces::resolve_commit_targets(
            &mut self.memory,
            &mut self.reservations,
            &mut self.spaces,
            space,
        );
        // A window that never mapped logged its break at create.
        let _ = mem.unmap_window(window_base(id));
    }

    /// Hand each access the batch from `space` made into a window to
    /// the window's unit.
    pub(super) fn route_raw_spu_window_accesses(
        &mut self,
        space: AddressSpaceId,
        effects: &[Effect],
    ) {
        let windows: Vec<Window> = self
            .raw_spu_windows()
            .into_iter()
            .filter(|w| w.space == space)
            .collect();
        if windows.is_empty() {
            return;
        }
        for effect in effects {
            let (range, bytes) = match effect {
                Effect::SharedWriteIntent { range, bytes, .. } => (*range, Some(bytes.bytes())),
                Effect::SharedReadIntent { range, .. } => (*range, None),
                _ => continue,
            };
            for window in &windows {
                let Some(offset) = window.offset_of(range) else {
                    continue;
                };
                match bytes {
                    Some(bytes) => self.write_raw_spu_window(window, offset, bytes),
                    None => self.read_raw_spu_window(window, offset, range.length()),
                }
            }
        }
    }

    /// Land a local-store write in the unit's local store, or run each
    /// whole word of a problem-state write as a register write.
    fn write_raw_spu_window(&mut self, window: &Window, offset: u64, bytes: &[u8]) {
        let end = offset + bytes.len() as u64;
        if end <= SPU_LS_SIZE as u64 {
            let landed = self
                .registry
                .get_mut(window.unit)
                .map(|spu| spu.land_local_store(offset as u32, bytes));
            if !matches!(landed, Some(Ok(()))) {
                self.log_window_refusal(window, offset, format_args!("{landed:?}"));
            }
            // `end` is inside the local store, so both bounds fit a u32.
            let reached = ByteRange::contiguous_u32(offset as u32, bytes.len() as u32);
            self.last_problem_state_accesses
                .push((window.unit, reached));
            return;
        }
        let Some(first) = offset.checked_sub(raw_window::PROBLEM_STATE) else {
            return self.log_window_refusal(
                window,
                offset,
                format_args!("a {}-byte store runs past the local store", bytes.len()),
            );
        };
        self.note_problem_state_access(window.unit);
        // Each register takes a whole aligned word.
        if !first.is_multiple_of(4) || !bytes.len().is_multiple_of(4) {
            self.log_window_refusal(
                window,
                offset,
                format_args!("a {}-byte store is not whole aligned words", bytes.len()),
            );
        }
        for (i, word) in bytes.chunks_exact(4).enumerate() {
            let register = first + 4 * i as u64;
            if !register.is_multiple_of(4) {
                continue;
            }
            let value = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            self.write_raw_spu_register(window, register as u32, value);
        }
    }

    /// A whole-word write of `value` to the problem-state register at
    /// `register`. A word that names no register the model runs lands
    /// in the window only, and the runtime logs the refusal.
    fn write_raw_spu_register(&mut self, window: &Window, register: u32, value: u32) {
        let unit = window.unit;
        let refusal = match register {
            SPU_IN_MBOX_OFFSET => self.write_unit_in_mbox(unit, value).err(),
            SPU_SIG_NOTIFY_1_OFFSET => self
                .write_unit_signal(unit, SignalNotifier::One, value)
                .err(),
            SPU_SIG_NOTIFY_2_OFFSET => self
                .write_unit_signal(unit, SignalNotifier::Two, value)
                .err(),
            // [CBEA p:95 s:8.5.3] a write of SPU_NPC while the SPU runs is ignored.
            SPU_NPC_OFFSET => match self.write_unit_npc(unit, value) {
                Err(ProblemStateError::Running) => None,
                other => other.err(),
            },
            SPU_RUN_CNTL_OFFSET => match value & 3 {
                0 => self.request_unit_stop(unit).err(),
                // A run request to a running SPU changes nothing.
                1 => match self.restart_unit(unit) {
                    Ok(()) => {
                        self.step_woke_others = true;
                        None
                    }
                    Err(RestartError::NotStopped) => None,
                    Err(err) => {
                        return self.log_window_refusal(
                            window,
                            raw_window::PROBLEM_STATE + u64::from(register),
                            format_args!("{err}"),
                        );
                    }
                },
                // [CBEA p:92 s:8.5.1] 10 is an isolation exit request and
                // 11 an isolation load request; CellGov models no isolation.
                isolation => {
                    return self.log_window_refusal(
                        window,
                        raw_window::PROBLEM_STATE + u64::from(register),
                        format_args!("isolation run control {isolation}"),
                    );
                }
            },
            unmodeled => {
                return self.log_window_refusal(
                    window,
                    raw_window::PROBLEM_STATE + u64::from(unmodeled),
                    format_args!("the model writes no register at 0x{unmodeled:x}"),
                );
            }
        };
        if let Some(err) = refusal {
            self.log_window_refusal(
                window,
                raw_window::PROBLEM_STATE + u64::from(register),
                format_args!("{err}"),
            );
        }
    }

    /// A load of `len` bytes at `offset` in `window`. A load that covers
    /// `SPU_Out_Mbox` takes the message it read.
    fn read_raw_spu_window(&mut self, window: &Window, offset: u64, len: u64) {
        let out_mbox = raw_window::PROBLEM_STATE + u64::from(SPU_OUT_MBOX_OFFSET);
        if offset <= out_mbox && offset + len >= out_mbox + 4 {
            self.note_problem_state_access(window.unit);
            if let Err(err) = self.read_unit_out_mbox(window.unit) {
                self.log_window_refusal(window, out_mbox, format_args!("{err}"));
            }
        }
    }

    /// Record that this step reached a register of `unit`'s
    /// problem-state area, for [`Runtime::last_problem_state_accesses`].
    pub(super) fn note_problem_state_access(&mut self, unit: UnitId) {
        // Both bounds sit inside the 1 MiB window, so they fit a u32.
        const AREA: ByteRange = ByteRange::contiguous_u32(
            raw_window::PROBLEM_STATE as u32,
            (raw_window::STRIDE - raw_window::PROBLEM_STATE) as u32,
        );
        self.last_problem_state_accesses.push((unit, AREA));
    }

    fn log_window_refusal(&mut self, window: &Window, offset: u64, why: std::fmt::Arguments<'_>) {
        self.lv2_host.log_invariant_break(
            "runtime.raw_spu_window_access_refused",
            format_args!(
                "Raw SPU {} ({:?}) refused the window access at offset 0x{offset:x}: {why}",
                window.id, window.unit
            ),
        );
    }

    /// Rewrite every window's registers from its unit.
    ///
    /// `stepped` names the unit whose step this commit closes, and
    /// `stopped` says that step ended at a stop. A Raw SPU that stepped
    /// latches the interrupts its step raised, and the runtime rewrites
    /// its local-store pages that changed. When `dma_landed` says a
    /// transfer completed in this commit, the runtime rewrites the
    /// changed local-store pages of every window.
    pub(super) fn project_raw_spu_windows(
        &mut self,
        stepped: UnitId,
        stopped: bool,
        dma_landed: bool,
    ) {
        for window in self.raw_spu_windows() {
            let own_step = stepped == window.unit;
            if own_step {
                self.latch_raw_spu_interrupts(&window, stopped);
            }
            if own_step || dma_landed {
                self.project_raw_spu_local_store(&window);
            }
            self.project_raw_spu_registers(&window);
        }
    }

    /// Latch the status bits the Raw SPU's step raised: class 2 M for a
    /// write to its interrupt mailbox and S for a stop-and-signal, and
    /// class 0 SE for a stop at an invalid instruction or channel access.
    fn latch_raw_spu_interrupts(&mut self, window: &Window, stopped: bool) {
        let mut stat = 0;
        if let Some(spu) = self.registry.get_mut(window.unit) {
            if spu.take_out_intr_mbox_interrupt() {
                stat |= raw_spu::INT_STAT_MAILBOX;
            }
        }
        let status = self
            .unit_stop_registers(window.unit)
            .map_or(0, |StopRegisters { status, .. }| status);
        if stopped && status & SPU_STATUS_P != 0 {
            stat |= raw_spu::INT_STAT_STOP_AND_SIGNAL;
        }
        let error = if stopped && status & (SPU_STATUS_I | SPU_STATUS_C) != 0 {
            raw_spu::INT_STAT_SPU_ERROR
        } else {
            0
        };
        for (class, bits) in [
            (raw_spu::CLASS_APPLICATION, stat),
            (raw_spu::CLASS_ERROR, error),
        ] {
            if bits != 0 && !self.lv2_host.raise_raw_spu_int_stat(window.id, class, bits) {
                self.lv2_host.log_invariant_break(
                    "runtime.raw_spu_interrupt_lost",
                    format_args!("Raw SPU {} left the table before its interrupt", window.id),
                );
            }
        }
    }

    /// Rewrite the registers the window shows: `SPU_Status`,
    /// `SPU_Mbox_Stat`, and `SPU_NPC` and `SPU_Out_Mbox` while they hold
    /// a value. A register with none keeps the word it last showed.
    ///
    /// [CBEA p:100 s:8.6.3] SPU_Mbox_Stat holds the interrupt mailbox count in bits 8:15, the free inbound entries in bits 16:23 and the outbound count in bits 24:31.
    /// [CBEA p:98 s:8.6.1] a read of an empty outbound mailbox returns undefined data.
    fn project_raw_spu_registers(&mut self, window: &Window) {
        let unit = window.unit;
        let status = self.unit_spu_status(unit).unwrap_or(0);
        let npc = self.unit_stop_registers(unit).map(|regs| regs.npc);
        let mailboxes = self
            .registry
            .get(unit)
            .and_then(|spu| spu.out_mailboxes())
            .unwrap_or_default();
        let queued = self
            .mailbox_registry
            .get(MailboxId::new(unit.raw()))
            .map_or(0, |mailbox| mailbox.len() as u32);
        let mbox_stat = u32::from(mailboxes.out_mbox.is_some())
            | SPU_IN_MBOX_DEPTH.saturating_sub(queued) << 8
            | u32::from(mailboxes.out_intr_mbox.is_some()) << 16;
        let words = [
            (SPU_STATUS_OFFSET, Some(status)),
            (SPU_NPC_OFFSET, npc),
            (SPU_MBOX_STAT_OFFSET, Some(mbox_stat)),
            (SPU_OUT_MBOX_OFFSET, mailboxes.out_mbox),
        ];
        for (offset, value) in words {
            if let Some(value) = value {
                let addr = window.register(offset);
                self.project_into_window(window, addr, &value.to_be_bytes());
            }
        }
    }

    /// Rewrite each local-store page of the window that differs from the
    /// unit's local store.
    fn project_raw_spu_local_store(&mut self, window: &Window) {
        let Some(Ok(ls)) = self
            .registry
            .get(window.unit)
            .map(|spu| spu.read_local_store(0, SPU_LS_SIZE as u32))
        else {
            return;
        };
        for (page, bytes) in ls.chunks(LS_PAGE as usize).enumerate() {
            let addr = window.base + page as u64 * u64::from(LS_PAGE);
            self.project_into_window(window, addr, bytes);
        }
    }

    /// Write `bytes` at `addr` in the window's space, unless the window
    /// already holds them.
    fn project_into_window(&mut self, window: &Window, addr: u64, bytes: &[u8]) {
        let range = ByteRange::contiguous_u32(addr as u32, bytes.len() as u32);
        let held = self
            .space_memory(window.space)
            .ok()
            .and_then(|mem| mem.read(range))
            .is_some_and(|held| held == bytes);
        if held {
            return;
        }
        if let Err(err) = self.host_write(
            HostWriter::SpuProblemState,
            window.space,
            range,
            bytes,
            None,
        ) {
            self.lv2_host.log_invariant_break(
                "runtime.raw_spu_window_projection_failed",
                format_args!(
                    "Raw SPU {}'s window at 0x{addr:x} refused its projection: {err}",
                    window.id
                ),
            );
        }
    }
}
