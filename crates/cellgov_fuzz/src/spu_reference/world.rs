//! The scripted outside world around one SPU. The vector-set replay and
//! the SPU sequence campaign run against it. It holds:
//!
//! - main storage;
//! - a second SPU in the thread window;
//! - the reservation table;
//! - the MFC queue;
//! - the inbound mailbox.
//!
//! It also carries out the problem-state operations the PPU performs.
//!
//! The world completes each MFC command in the step that queued it, so
//! a run sees one legal completion order. Guest time advances by each
//! step's cost, or by the ticks a caller passes to [`World::advance`]
//! when it executes instructions itself. The SPU's outbound interrupt
//! mailbox has a reader: LV2 reads it for an SPU thread, and the PPU
//! for a raw SPU.

use cellgov_dma::{DmaCompletion, DmaDirection, MfcCommandError};
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionUnit, ProblemStateError, RestartError, SignalNotifier, UnitStatus,
    YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;
use cellgov_ps3_abi::hw::spu::{
    SPU_IN_MBOX_OFFSET, SPU_LS_SIZE, SPU_SIG_NOTIFY_1_OFFSET, SPU_SIG_NOTIFY_2_OFFSET,
};
use cellgov_ps3_abi::lv2::spu::thread_window;
use cellgov_spu::state::SpuState;
use cellgov_spu::{SpuExecutionUnit, SpuSnapshot};
use cellgov_sync::{ReservationTable, ReservedLine};
use cellgov_time::{Budget, GuestTicks};

use super::set_types::{
    SpuReferenceEffect, SpuReferenceInvalidCommand, SpuReferencePpuAction, SpuReferencePpuRefusal,
    SpuReferencePpuResult,
};
use super::types::SpuReferenceError;

/// The world's SPU. It takes slot 0 of the SPU thread window.
pub(crate) const OWN: UnitId = UnitId::new(0);
/// The world's second SPU. It takes slot 1.
const PEER: UnitId = UnitId::new(1);

pub(crate) fn world_error(what: &'static str) -> SpuReferenceError {
    SpuReferenceError::World { what }
}

/// Where a transfer into the SPU thread window lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowTarget {
    /// A range of a unit's local store.
    LocalStore { unit: UnitId, lsa: u32 },
    /// One of a unit's signal-notification registers.
    Signal {
        unit: UnitId,
        register: SignalNotifier,
    },
    /// A unit's inbound mailbox.
    InboundMailbox { unit: UnitId },
}

/// Where `c` lands in the window, or `None` outside it. A slot with no
/// SPU, or an offset no transfer reaches, refuses the transfer.
///
/// [CBEA p:37 s:3.2] a local store can be aliased into the main storage domain.
/// [CBEA p:38 s:3.2.1] an MFC effective address can name an aliased local store, its own included.
fn window_target(
    has_peer: bool,
    c: &DmaCompletion,
) -> Option<Result<WindowTarget, MfcCommandError>> {
    let range = match c.direction() {
        DmaDirection::Put => c.destination(),
        DmaDirection::Get => c.source(),
    };
    let (ea, len) = (range.start().raw(), range.length());
    if len == 0 || !(thread_window::BASE..=u64::from(u32::MAX)).contains(&ea) {
        return None;
    }
    let refused = Some(Err(MfcCommandError::DataStorage { ea }));
    let unit = match (ea - thread_window::BASE) / thread_window::STRIDE {
        0 => OWN,
        1 if has_peer => PEER,
        _ => return refused,
    };
    let offset = (ea - thread_window::BASE) % thread_window::STRIDE;
    if offset + len <= SPU_LS_SIZE as u64 {
        return Some(Ok(WindowTarget::LocalStore {
            unit,
            lsa: offset as u32,
        }));
    }
    if c.direction() != DmaDirection::Put || len != 4 {
        return refused;
    }
    let register = offset.checked_sub(thread_window::PROBLEM_STATE);
    Some(Ok(match register.map(|r| r as u32) {
        Some(SPU_SIG_NOTIFY_1_OFFSET) => WindowTarget::Signal {
            unit,
            register: SignalNotifier::One,
        },
        Some(SPU_SIG_NOTIFY_2_OFFSET) => WindowTarget::Signal {
            unit,
            register: SignalNotifier::Two,
        },
        Some(SPU_IN_MBOX_OFFSET) => WindowTarget::InboundMailbox { unit },
        _ => return refused,
    }))
}

/// The fault a main-storage transfer raises, or `None`: a data-segment
/// fault for a range no region backs, as the `spu_ea_bound` console
/// capture shows, and a data-storage fault for a backed range the access
/// may not use.
///
/// [CBEA p:120 s:9.1.7] a segment fault raises the MFC data-segment interrupt; a mapping fault or a protection violation raises the MFC data-storage interrupt.
fn translation_fault(
    memory: &GuestMemory,
    c: &DmaCompletion,
    payloaded: bool,
) -> Option<MfcCommandError> {
    let check = |range: ByteRange, write: bool| {
        if range.length() == 0 {
            return None;
        }
        let ea = range.start().raw();
        let backed =
            ea + (range.length() - 1) <= CELL_EA_LIMIT && memory.read_checked(range).is_ok();
        if !backed {
            return Some(MfcCommandError::DataSegment { ea });
        }
        let permitted = !write
            || memory
                .validate_write(range, range.length() as usize)
                .is_ok();
        (!permitted).then_some(MfcCommandError::DataStorage { ea })
    };
    let request = c.request();
    request
        .main_storage_read(payloaded)
        .and_then(|range| check(range, false))
        .or_else(|| {
            request
                .main_storage_write()
                .and_then(|range| check(range, true))
        })
}

/// The view of the world a step of the SPU at slot 0 starts from.
fn context<'a>(
    memory: &'a GuestMemory,
    reservations: &'a ReservationTable,
    inbox: &'a [u32],
    queue: &cellgov_dma::DmaQueue,
    now: GuestTicks,
) -> ExecutionContext<'a> {
    let (occupancy, tags) = queue.issuer_view(OWN);
    let (barrier_tags, queue_barrier) = queue.issuer_barriers(OWN);
    let oldest = queue.pending_sequenced().map(|(seq, _)| seq).min();
    ExecutionContext::new(memory)
        .with_current_tick(now)
        .with_reservations(reservations)
        .with_inbound_mailbox(inbox)
        .with_outstanding_dma_tags(tags)
        .with_list_stall_tags(queue.stall_notify_tags(OWN))
        .with_dma_queue_occupancy(occupancy)
        .with_dma_barriers(barrier_tags, queue_barrier)
        .with_mfc_transfer_view(queue.next_sequence(), oldest)
}

/// The queued commands of the SPU at slot 0 that its next command can wait
/// behind.
fn ordering_view(queue: &cellgov_dma::DmaQueue) -> cellgov_exec::MfcOrderingView {
    cellgov_exec::MfcOrderingView::of_issuer(queue, OWN)
}

/// The scripted outside world around one SPU.
pub(crate) struct World {
    pub(crate) unit: SpuExecutionUnit,
    pub(crate) peer: Option<SpuExecutionUnit>,
    memory: GuestMemory,
    /// Each main-storage region's base and length.
    regions: Vec<(u64, u64)>,
    reservations: ReservationTable,
    queue: cellgov_dma::DmaQueue,
    inbox: Vec<u32>,
    pub(crate) peer_inbox: Vec<u32>,
    pub(crate) effects: Vec<SpuReferenceEffect>,
    pub(crate) exceptions: Vec<SpuReferenceInvalidCommand>,
    pub(crate) ppu_results: Vec<SpuReferencePpuResult>,
    pub(crate) fault: Option<FaultKind>,
    /// A store the world could not commit refused the step, as the
    /// commit pipeline refuses a batch.
    pub(crate) refused: bool,
    /// The guest time the next step starts at.
    now: GuestTicks,
}

impl World {
    /// A world whose SPU starts from `loaded`.
    ///
    /// Each `(base, bytes)` run becomes one main-storage region. A `Some`
    /// `peer_ls` adds a second SPU that holds that local store.
    ///
    /// # Errors
    ///
    /// Refuses runs `GuestMemory` cannot map, and a `peer_ls` whose
    /// length is not `SPU_LS_SIZE`.
    pub(crate) fn new(
        loaded: &SpuState,
        memory_runs: &[(u64, Vec<u8>)],
        peer_ls: Option<Vec<u8>>,
    ) -> Result<Self, SpuReferenceError> {
        Self::with_local_store(loaded, Vec::new(), memory_runs, peer_ls)
    }

    /// [`Self::new`], with the SPU's local store in `ls`'s buffer.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub(crate) fn with_local_store(
        loaded: &SpuState,
        ls: Vec<u8>,
        memory_runs: &[(u64, Vec<u8>)],
        peer_ls: Option<Vec<u8>>,
    ) -> Result<Self, SpuReferenceError> {
        let mut unit = SpuExecutionUnit::restored(
            OWN,
            SpuSnapshot {
                state: loaded.clone_with_local_store(ls),
                status: if loaded.stop.is_some() {
                    UnitStatus::Finished
                } else {
                    UnitStatus::Runnable
                },
                stall: None,
                pipeline: cellgov_spu::timing::SpuPipeline::new(),
            },
        );
        unit.connect_out_intr_mbox()
            .map_err(|_| world_error("the interrupt mailbox"))?;
        let mut regions = Vec::new();
        let mut layout = Vec::new();
        for (base, bytes) in memory_runs {
            regions.push(Region::new(
                *base,
                bytes.len(),
                "reference",
                PageSize::Page4K,
            ));
            layout.push((*base, bytes.len() as u64));
        }
        let mut memory =
            GuestMemory::from_regions(regions).map_err(|_| world_error("world.memory"))?;
        for (base, bytes) in memory_runs {
            let range = ByteRange::new(GuestAddr::new(*base), bytes.len() as u64)
                .ok_or(world_error("world.memory"))?;
            memory
                .apply_commit(range, bytes)
                .map_err(|_| world_error("world.memory"))?;
        }
        if peer_ls.as_ref().is_some_and(|ls| ls.len() != SPU_LS_SIZE) {
            return Err(world_error("the peer local store"));
        }
        let peer = peer_ls.map(|ls| {
            let mut unit = SpuExecutionUnit::new(PEER);
            unit.state_mut().ls.copy_from_slice(&ls);
            unit
        });
        let mut reservations = ReservationTable::new();
        if let Some(line) = loaded.reservation() {
            reservations.insert_or_replace(OWN, line);
        }
        Ok(Self {
            unit,
            peer,
            memory,
            regions: layout,
            reservations,
            queue: cellgov_dma::DmaQueue::new(),
            inbox: loaded.channels.in_mbox.clone(),
            peer_inbox: Vec::new(),
            effects: Vec::new(),
            exceptions: Vec::new(),
            ppu_results: Vec::new(),
            fault: None,
            refused: false,
            now: GuestTicks::ZERO,
        })
    }

    /// The SPU's state, which outlives the world.
    pub(crate) fn into_state(self) -> SpuState {
        self.unit.into_state()
    }

    /// Each region's bytes, in region order.
    pub(crate) fn memory_image(&self) -> Result<Vec<(u64, Vec<u8>)>, SpuReferenceError> {
        self.regions
            .iter()
            .map(|&(base, len)| {
                let range =
                    ByteRange::new(GuestAddr::new(base), len).ok_or(world_error("world.memory"))?;
                self.memory
                    .read(range)
                    .map(|bytes| (base, bytes.to_vec()))
                    .ok_or(world_error("world.memory"))
            })
            .collect()
    }

    /// Carries out one problem-state operation and records its result.
    pub(crate) fn apply(&mut self, action: SpuReferencePpuAction) {
        let register = |number: u8| {
            if number == 1 {
                SignalNotifier::One
            } else {
                SignalNotifier::Two
            }
        };
        let done = |result: Result<(), ProblemStateError>| match result {
            Ok(()) => SpuReferencePpuResult::Done,
            Err(error) => SpuReferencePpuResult::Refused {
                reason: match error {
                    ProblemStateError::Running => SpuReferencePpuRefusal::Running,
                    ProblemStateError::Refused => SpuReferencePpuRefusal::Refused,
                    ProblemStateError::NoProblemState
                    | ProblemStateError::UnknownUnit
                    | ProblemStateError::Retired => SpuReferencePpuRefusal::Other,
                },
            },
        };
        let result = match action {
            SpuReferencePpuAction::InMbox { value } => {
                self.inbox.push(value);
                SpuReferencePpuResult::Done
            }
            SpuReferencePpuAction::Signal {
                register: number,
                value,
            } => done(self.unit.write_signal(register(number), value)),
            SpuReferencePpuAction::SignalMode {
                register: number,
                logical_or,
            } => done(
                self.unit
                    .set_signal_logical_or(register(number), logical_or),
            ),
            SpuReferencePpuAction::ReadOutMbox => match self.unit.read_out_mbox() {
                Ok(value) => SpuReferencePpuResult::Read { value },
                Err(error) => done(Err(error)),
            },
            SpuReferencePpuAction::ReadOutIntrMbox => match self.unit.read_out_intr_mbox() {
                Ok(value) => SpuReferencePpuResult::Read { value },
                Err(error) => done(Err(error)),
            },
            // [CBEA p:164 s:9.12.10] the reservation is lost when another processor or device modifies the line.
            SpuReferencePpuAction::StoreWord { ea, value } => {
                let stored = ByteRange::new(GuestAddr::new(ea), 4).is_some_and(|range| {
                    self.memory
                        .apply_commit(range, &value.to_be_bytes())
                        .is_ok()
                });
                if stored {
                    self.reservations.clear_covering(ea, 4, None);
                    SpuReferencePpuResult::Done
                } else {
                    SpuReferencePpuResult::Refused {
                        reason: SpuReferencePpuRefusal::Other,
                    }
                }
            }
            SpuReferencePpuAction::StopRequest => {
                let waiting = self.unit.channel_stall().is_some();
                done(self.unit.request_stop(waiting))
            }
            SpuReferencePpuAction::WriteNpc { value } => done(self.unit.write_npc(value)),
            SpuReferencePpuAction::Restart => match self.unit.restart() {
                Ok(()) => SpuReferencePpuResult::Done,
                Err(RestartError::NotStopped) => SpuReferencePpuResult::Refused {
                    reason: SpuReferencePpuRefusal::NotStopped,
                },
                Err(_) => SpuReferencePpuResult::Refused {
                    reason: SpuReferencePpuRefusal::Other,
                },
            },
        };
        self.ppu_results.push(result);
    }

    /// Runs one instruction through the execution unit, services what it
    /// emitted, and completes every command it queued.
    pub(crate) fn step(&mut self) -> Result<(), SpuReferenceError> {
        let mut effects = Vec::new();
        let ctx = context(
            &self.memory,
            &self.reservations,
            &self.inbox,
            &self.queue,
            self.now,
        );
        let result = self
            .unit
            .run_until_yield(Budget::new(1), &ctx, &mut effects);
        self.advance(result.consumed_cost.into())?;
        // A faulting step's effects are discarded, as the commit pipeline
        // discards them.
        if result.yield_reason == YieldReason::Fault {
            self.fault = result.fault;
            return Ok(());
        }
        self.settle(&effects)
    }

    /// Updates the SPU's state from the world, as a step entry does.
    ///
    /// A caller that executes the next instruction itself calls this
    /// first.
    pub(crate) fn enter(&mut self) {
        let ctx = context(
            &self.memory,
            &self.reservations,
            &self.inbox,
            &self.queue,
            self.now,
        );
        cellgov_spu::step::enter_step(self.unit.state_mut(), OWN, &ctx);
    }

    /// Completes the atomic read the SPU asked for, through
    /// [`cellgov_spu::step::complete_memory_read`].
    ///
    /// The caller advances the program counter and passes the appended
    /// effects to [`World::settle`].
    pub(crate) fn complete_read(
        &mut self,
        ea: u64,
        lsa: u32,
        size: u32,
        acquire_line: Option<u64>,
        effects: &mut Vec<Effect>,
    ) -> cellgov_spu::step::MemoryReadCompletion {
        cellgov_spu::step::complete_memory_read(
            self.unit.state_mut(),
            &self.memory,
            OWN,
            ea,
            lsa,
            size,
            acquire_line,
            effects,
        )
    }

    /// Advances guest time by `ticks`.
    pub(crate) fn advance(&mut self, ticks: GuestTicks) -> Result<(), SpuReferenceError> {
        self.now = self
            .now
            .checked_add(ticks)
            .ok_or(world_error("guest time"))?;
        Ok(())
    }

    /// Records and commits a step's effects, then completes every command
    /// they queued.
    pub(crate) fn settle(&mut self, effects: &[Effect]) -> Result<(), SpuReferenceError> {
        // A clock read tells a scheduler which steps it cannot reorder.
        // The world runs one unit, so the effect changes nothing here
        // and a vector does not list it.
        for effect in effects
            .iter()
            .filter(|e| !matches!(e, Effect::ClockRead { .. }))
        {
            self.effects.push(effect.into());
            self.service(effect);
        }
        self.drain()
    }

    /// Commits one effect to the world.
    fn service(&mut self, effect: &Effect) {
        match effect {
            Effect::DmaEnqueue { request, payload } => {
                self.queue.enqueue(
                    DmaCompletion::new(*request, GuestTicks::ZERO),
                    payload.clone(),
                );
            }
            Effect::MfcInvalidCommand { issuer, command } => {
                self.queue
                    .enqueue_invalid(GuestTicks::ZERO, *issuer, *command);
            }
            Effect::SharedWriteIntent {
                range,
                bytes,
                source,
                ..
            } => self.commit(*range, bytes.bytes(), *source),
            Effect::ConditionalStore {
                range,
                bytes,
                source,
                ..
            } => {
                self.commit(*range, bytes.bytes(), *source);
                self.reservations.remove_if_present(*source);
            }
            Effect::ReservationAcquire { line_addr, source } => {
                self.reservations
                    .insert_or_replace(*source, ReservedLine::containing(*line_addr));
            }
            Effect::MailboxPop { message, .. } if self.inbox.first() == Some(&message.raw()) => {
                self.inbox.remove(0);
            }
            _ => {}
        }
    }

    /// Writes `bytes` to main storage and clears every other unit's
    /// reservation over them.
    fn commit(&mut self, range: ByteRange, bytes: &[u8], source: UnitId) {
        if self.memory.apply_commit(range, bytes).is_err() {
            self.refused = true;
            return;
        }
        self.reservations
            .clear_covering(range.start().raw(), range.length(), Some(source));
    }

    /// Completes every queued command, then every command that a released
    /// held list queues.
    ///
    /// Each pass completes at least one command, and a list queues
    /// finitely many, so the loop ends.
    fn drain(&mut self) -> Result<(), SpuReferenceError> {
        loop {
            let has_peer = self.peer.is_some();
            let memory = &self.memory;
            let due = self
                .queue
                .process_due_translating(GuestTicks::ZERO, |c, payloaded| {
                    match window_target(has_peer, c) {
                        Some(target) => target.err(),
                        None => translation_fault(memory, c, payloaded),
                    }
                });
            for raised in &due.raised {
                self.exceptions.push((&raised.command).into());
            }
            for (c, payload) in &due.completions {
                self.land(c, payload.as_deref())?;
            }
            if due.completions.is_empty() {
                return Ok(());
            }
            let started = self.unit.start_held_lists(ordering_view(&self.queue));
            if started.is_empty() {
                return Ok(());
            }
            for effect in &started {
                self.effects.push(effect.into());
                self.service(effect);
            }
        }
    }

    fn unit_mut(&mut self, id: UnitId) -> Result<&mut SpuExecutionUnit, SpuReferenceError> {
        if id == OWN {
            Ok(&mut self.unit)
        } else {
            self.peer.as_mut().ok_or(world_error("the peer SPU"))
        }
    }

    /// Lands one completed transfer.
    fn land(&mut self, c: &DmaCompletion, payload: Option<&[u8]>) -> Result<(), SpuReferenceError> {
        let len = c.length();
        if len == 0 {
            return Ok(());
        }
        let target = window_target(self.peer.is_some(), c);
        let fail = |_| world_error("a completed transfer");
        match c.direction() {
            DmaDirection::Get => {
                let mut bytes = match target {
                    Some(Ok(WindowTarget::LocalStore { unit, lsa })) => self
                        .unit_mut(unit)?
                        .read_local_store(lsa, len as u32)
                        .map_err(fail)?,
                    Some(_) => return Ok(()),
                    None => self
                        .memory
                        .read(c.source())
                        .ok_or(world_error("a get's source"))?
                        .to_vec(),
                };
                crate::seeded::world_get(&mut bytes);
                // The destination is a local-store offset, below 2^32.
                let lsa = c.destination().start().raw() as u32;
                self.unit.land_local_store(lsa, &bytes).map_err(fail)
            }
            DmaDirection::Put => {
                let bytes = match payload {
                    Some(bytes) => bytes.to_vec(),
                    None if c.request().local_store_source() => self
                        .unit
                        .read_local_store(c.source().start().raw() as u32, len as u32)
                        .map_err(fail)?,
                    None => self
                        .memory
                        .read(c.source())
                        .ok_or(world_error("a put's source"))?
                        .to_vec(),
                };
                let mut word = [0; 4];
                for (slot, byte) in word.iter_mut().zip(&bytes) {
                    *slot = *byte;
                }
                let word = u32::from_be_bytes(word);
                match target {
                    Some(Ok(WindowTarget::LocalStore { unit, lsa })) => self
                        .unit_mut(unit)?
                        .land_local_store(lsa, &bytes)
                        .map_err(fail),
                    Some(Ok(WindowTarget::Signal { unit, register })) => self
                        .unit_mut(unit)?
                        .write_signal(register, word)
                        .map_err(fail),
                    Some(Ok(WindowTarget::InboundMailbox { unit })) => {
                        if unit == OWN {
                            self.inbox.push(word);
                        } else {
                            self.peer_inbox.push(word);
                        }
                        Ok(())
                    }
                    Some(Err(_)) => Ok(()),
                    None => {
                        self.commit(c.destination(), &bytes, c.issuer());
                        Ok(())
                    }
                }
            }
        }
    }
}
