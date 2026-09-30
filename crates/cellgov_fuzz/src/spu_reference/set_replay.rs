//! Replay of a vector set: each vector runs one step at a time against
//! a scripted outside world, which services every transfer, store,
//! reservation and mailbox read the SPU makes.
//!
//! The world completes each MFC command in the step that queued it, so
//! a vector sees one legal completion order. A vector that depends on
//! another order states the choices as a set of legal values.

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
use cellgov_spu::stop::SpuStop;
use cellgov_spu::{SpuExecutionUnit, SpuSnapshot};
use cellgov_sync::{ReservationTable, ReservedLine};
use cellgov_time::{Budget, GuestTicks};

use super::set_compare::{compare_vector, SpuVectorComparison, SpuVectorObservation};
use super::set_convert::{parse_hex_bytes, write_runs};
use super::set_types::{
    SpuReferenceEffect, SpuReferenceEnd, SpuReferenceInvalidCommand, SpuReferencePpuAction,
    SpuReferencePpuRefusal, SpuReferencePpuResult, SpuReferenceSet, SpuReferenceStart,
    SpuReferenceVector,
};
use super::types::SpuReferenceError;
use super::validate::{parse_fpscr, parse_index, parse_register};

/// The replayed SPU. It takes slot 0 of the SPU thread window.
const OWN: UnitId = UnitId::new(0);
/// The world's second SPU. It takes slot 1.
const PEER: UnitId = UnitId::new(1);

/// One vector's replay: what the SPU did and how it compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuVectorReplay {
    /// The vector's name.
    pub name: String,
    /// The replayed SPU's whole context at the end.
    pub snapshot: SpuSnapshot,
    /// How the replay ended.
    pub end: SpuReferenceEnd,
    /// Every effect, in emission order.
    pub effects: Vec<SpuReferenceEffect>,
    /// One result per problem-state operation.
    pub ppu_results: Vec<SpuReferencePpuResult>,
    /// Commands the MFC refused when its queue reached them.
    pub mfc_exceptions: Vec<SpuReferenceInvalidCommand>,
    /// Steps the SPU ran.
    pub steps: u32,
    /// The comparison against the expected end.
    pub comparison: SpuVectorComparison,
}

/// Replays every vector of a set without a device, network or external
/// runner.
///
/// [Martignoni2009 p:127 s:2.3] Both CPUs start from the same synthetic state and execute the case; the comparison reads only their final states.
pub fn replay_reference_set(
    set: &SpuReferenceSet,
) -> Result<Vec<SpuVectorReplay>, SpuReferenceError> {
    set.validate()?;
    set.vectors
        .iter()
        .enumerate()
        .map(|(index, vector)| {
            replay_vector(vector).map_err(|source| SpuReferenceError::InVector {
                index,
                source: Box::new(source),
            })
        })
        .collect()
}

fn world_error(what: &'static str) -> SpuReferenceError {
    SpuReferenceError::World { what }
}

fn replay_vector(vector: &SpuReferenceVector) -> Result<SpuVectorReplay, SpuReferenceError> {
    let loaded = start_state(&vector.initial_state, &vector.words)?;
    let mut world = World::new(vector, &loaded)?;
    let initial_memory = world.memory_image()?;
    let initial_peer_ls = world.peer.as_ref().map(|peer| peer.state().ls.clone());
    let mut actions = vector.world.ppu.iter().peekable();
    let mut steps = 0;
    for step in 0..vector.step_limit {
        let mut landed = false;
        while let Some(write) = actions.next_if(|write| write.step == step) {
            world.apply(write.action);
            landed = true;
        }
        let pending = actions.peek().is_some();
        let runnable = world.unit.status() == UnitStatus::Runnable && !world.refused;
        // A stalled access runs again while an operation can still wake it.
        let stalled = world.unit.channel_stall().is_some() && !landed;
        if !runnable || (stalled && !pending) {
            if pending {
                continue;
            }
            break;
        }
        steps += 1;
        world.step()?;
    }
    let end = if world.refused {
        SpuReferenceEnd::Faulted { code: None }
    } else {
        match world.unit.status() {
            UnitStatus::Finished => SpuReferenceEnd::Stopped,
            UnitStatus::Faulted => SpuReferenceEnd::Faulted {
                code: match world.fault {
                    Some(FaultKind::Guest(code)) => Some(code),
                    Some(FaultKind::Validation) | None => None,
                },
            },
            UnitStatus::Runnable | UnitStatus::Blocked => match world.unit.channel_stall() {
                Some(stall) => SpuReferenceEnd::Stalled {
                    channel: stall.channel,
                },
                None => SpuReferenceEnd::StepLimit,
            },
        }
    };
    let snapshot = world.unit.snapshot();
    let observation = SpuVectorObservation {
        loaded: &loaded,
        snapshot: &snapshot,
        end,
        effects: &world.effects,
        initial_memory: &initial_memory,
        memory: &world.memory_image()?,
        initial_peer_ls: initial_peer_ls.as_deref(),
        peer: world
            .peer
            .as_ref()
            .map(|peer| (peer.state(), world.peer_inbox.as_slice())),
        ppu_results: &world.ppu_results,
        mfc_exceptions: &world.exceptions,
    };
    let comparison = compare_vector(&vector.expected, &observation);
    Ok(SpuVectorReplay {
        name: vector.name.clone(),
        snapshot,
        end,
        effects: world.effects,
        ppu_results: world.ppu_results,
        mfc_exceptions: world.exceptions,
        steps,
        comparison,
    })
}

/// The start state, with the instruction words loaded.
fn start_state(start: &SpuReferenceStart, words: &[u32]) -> Result<SpuState, SpuReferenceError> {
    let invalid = |field| SpuReferenceError::Invalid { field };
    let mut state = SpuState::new();
    state.pc = start.pc;
    if let Some(lslr) = start.lslr {
        state.set_lslr(lslr);
    }
    if let Some(hex) = &start.fpscr {
        state.set_fpscr(parse_fpscr(hex).ok_or(invalid("initial_state.fpscr"))?);
    }
    for (index, hex) in &start.regs_hex {
        let index = parse_index(index, state.regs.as_array().len())
            .ok_or(invalid("initial_state.regs_hex"))?;
        state.set_reg(
            index,
            parse_register(hex).ok_or(invalid("initial_state.regs_hex"))?,
        );
    }
    write_runs(&mut state.ls, 0, &start.local_store);
    for (index, word) in words.iter().enumerate() {
        let offset = start.pc as usize + index * 4;
        state.ls[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
    }
    state.stop = start.stop.as_ref().map(SpuStop::from);
    state.set_interrupts_enabled(start.interrupts_enabled);
    state.set_srr0(start.srr0);
    if let Some(signals) = &start.signals {
        state.signals = [(&signals[0]).into(), (&signals[1]).into()];
    }
    if let Some(channels) = &start.channels {
        state.channels = channels
            .to_channels()
            .ok_or(invalid("initial_state.channels"))?;
    }
    state.set_reservation(start.reservation.map(ReservedLine::containing));
    Ok(state)
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

/// The fault a main-storage transfer raises, or `None`.
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
        if ea + (range.length() - 1) > CELL_EA_LIMIT {
            return Some(MfcCommandError::DataSegment { ea });
        }
        let translates = if write {
            memory
                .validate_write(range, range.length() as usize)
                .is_ok()
        } else {
            memory.read_checked(range).is_ok()
        };
        (!translates).then_some(MfcCommandError::DataStorage { ea })
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

/// The scripted outside world around the replayed SPU.
struct World {
    unit: SpuExecutionUnit,
    peer: Option<SpuExecutionUnit>,
    memory: GuestMemory,
    /// Each main-storage region's base and length.
    regions: Vec<(u64, u64)>,
    reservations: ReservationTable,
    queue: cellgov_dma::DmaQueue,
    inbox: Vec<u32>,
    peer_inbox: Vec<u32>,
    effects: Vec<SpuReferenceEffect>,
    exceptions: Vec<SpuReferenceInvalidCommand>,
    ppu_results: Vec<SpuReferencePpuResult>,
    fault: Option<FaultKind>,
    /// A store the world could not commit refused the step, as the
    /// commit pipeline refuses a batch.
    refused: bool,
}

impl World {
    fn new(vector: &SpuReferenceVector, loaded: &SpuState) -> Result<Self, SpuReferenceError> {
        let mut unit = SpuExecutionUnit::new(OWN);
        unit.restore(SpuSnapshot {
            state: loaded.clone(),
            status: if loaded.stop.is_some() {
                UnitStatus::Finished
            } else {
                UnitStatus::Runnable
            },
            stall: None,
        });
        let mut regions = Vec::new();
        let mut layout = Vec::new();
        for run in &vector.world.memory {
            let bytes = parse_hex_bytes(&run.hex).ok_or(world_error("world.memory"))?;
            regions.push(Region::new(
                run.at,
                bytes.len(),
                "reference",
                PageSize::Page4K,
            ));
            layout.push((run.at, bytes.len() as u64));
        }
        let mut memory =
            GuestMemory::from_regions(regions).map_err(|_| world_error("world.memory"))?;
        for run in &vector.world.memory {
            let bytes = parse_hex_bytes(&run.hex).ok_or(world_error("world.memory"))?;
            let range = ByteRange::new(GuestAddr::new(run.at), bytes.len() as u64)
                .ok_or(world_error("world.memory"))?;
            memory
                .apply_commit(range, &bytes)
                .map_err(|_| world_error("world.memory"))?;
        }
        let peer = vector.world.peer.as_ref().map(|peer| {
            let mut unit = SpuExecutionUnit::new(PEER);
            write_runs(&mut unit.state_mut().ls, 0, &peer.local_store);
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
        })
    }

    /// Each region's bytes, in region order.
    fn memory_image(&self) -> Result<Vec<(u64, Vec<u8>)>, SpuReferenceError> {
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
    fn apply(&mut self, action: SpuReferencePpuAction) {
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

    /// Runs one instruction, services what it emitted, and completes
    /// every command it queued.
    fn step(&mut self) -> Result<(), SpuReferenceError> {
        let (occupancy, tags) = self.queue.issuer_view(OWN);
        let oldest = self.queue.pending_sequenced().map(|(seq, _)| seq).min();
        let ctx = ExecutionContext::new(&self.memory)
            .with_reservations(&self.reservations)
            .with_inbound_mailbox(&self.inbox)
            .with_outstanding_dma_tags(tags)
            .with_list_stall_tags(self.queue.stall_notify_tags(OWN))
            .with_dma_queue_occupancy(occupancy)
            .with_mfc_transfer_view(self.queue.next_sequence(), oldest);
        let mut effects = Vec::new();
        let result = self
            .unit
            .run_until_yield(Budget::new(1), &ctx, &mut effects);
        // A faulting step's effects are discarded, as the commit pipeline
        // discards them.
        if result.yield_reason == YieldReason::Fault {
            self.fault = result.fault;
            return Ok(());
        }
        for effect in &effects {
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

    /// Completes every queued command.
    fn drain(&mut self) -> Result<(), SpuReferenceError> {
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
        Ok(())
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
                let bytes = match target {
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
