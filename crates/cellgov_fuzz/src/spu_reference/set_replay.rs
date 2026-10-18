//! Replay of a vector set: each vector runs one step at a time against
//! the scripted [`World`], which services every transfer, store,
//! reservation and mailbox read the SPU makes.
//!
//! The world completes each MFC command in the step that queued it, so
//! a vector sees one legal completion order. A vector that depends on
//! another order states the choices as a set of legal values.

use cellgov_effects::FaultKind;
use cellgov_exec::{ExecutionUnit, UnitStatus};
use cellgov_ps3_abi::hw::spu::SPU_LS_SIZE;
use cellgov_spu::state::SpuState;
use cellgov_spu::stop::SpuStop;
use cellgov_spu::SpuSnapshot;
use cellgov_sync::ReservedLine;

use super::set_compare::{compare_vector, SpuVectorComparison, SpuVectorObservation};
use super::set_convert::{parse_hex_bytes, write_runs};
use super::set_types::{
    SpuReferenceEffect, SpuReferenceEnd, SpuReferenceInvalidCommand, SpuReferencePpuResult,
    SpuReferenceSet, SpuReferenceStart, SpuReferenceVector,
};
use super::types::SpuReferenceError;
use super::validate::{parse_fpscr, parse_index, parse_register};
use super::world::{world_error, World};

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

fn replay_vector(vector: &SpuReferenceVector) -> Result<SpuVectorReplay, SpuReferenceError> {
    let loaded = start_state(&vector.initial_state, &vector.words)?;
    let memory = vector
        .world
        .memory
        .iter()
        .map(|run| {
            parse_hex_bytes(&run.hex)
                .map(|bytes| (run.at, bytes))
                .ok_or(world_error("world.memory"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let peer_ls = vector.world.peer.as_ref().map(|peer| {
        let mut ls = vec![0; SPU_LS_SIZE];
        write_runs(&mut ls, 0, &peer.local_store);
        ls
    });
    let mut world = World::new(&loaded, &memory, peer_ls)?;
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
            UnitStatus::Runnable | UnitStatus::Blocked | UnitStatus::Suspended => {
                match world.unit.channel_stall() {
                    Some(stall) => SpuReferenceEnd::Stalled {
                        channel: stall.channel,
                    },
                    None => SpuReferenceEnd::StepLimit,
                }
            }
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
