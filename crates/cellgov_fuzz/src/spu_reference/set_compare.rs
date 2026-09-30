//! Comparison of one replayed vector against its expected end.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_spu::state::SpuState;
use cellgov_spu::SpuSnapshot;

use crate::reference::ReferenceOmission;

use super::set_convert::write_runs;
use super::set_types::{
    SpuField, SpuReferenceChannelState, SpuReferenceEffect, SpuReferenceEnd, SpuReferenceFinal,
    SpuReferenceInvalidCommand, SpuReferencePpuResult, SpuReferenceSignal, SpuReferenceStop,
};
use super::validate::{parse_fpscr, parse_index, parse_register};

/// A compared component of a vector's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuVectorComponent {
    /// How the replay ended.
    End,
    /// The register bank.
    Registers,
    /// The local store.
    LocalStore,
    /// The program counter.
    ProgramCounter,
    /// The local storage limit register.
    Lslr,
    /// The FPSCR.
    Fpscr,
    /// The stopped state.
    Stop,
    /// The interrupt-enable state.
    InterruptsEnabled,
    /// SRR0.
    Srr0,
    /// The signal-notification registers.
    Signals,
    /// Every channel's data and count.
    Channels,
    /// The local reservation.
    Reservation,
    /// The emitted effects.
    Effects,
    /// Main storage.
    MainMemory,
    /// The other SPU.
    Peer,
    /// The problem-state operation results.
    PpuResults,
    /// The commands the MFC refused.
    MfcExceptions,
}

/// Result of comparing one vector.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpuVectorComparison {
    /// Components with a value or a set of legal values.
    pub compared: BTreeSet<SpuVectorComponent>,
    /// Components whose result matched no expected value.
    pub differences: BTreeSet<SpuVectorComponent>,
    /// Components whose result is legal but not the value CellGov
    /// documents choosing.
    pub unchosen: BTreeSet<SpuVectorComponent>,
    /// Components not compared, with the reason kind.
    pub unrepresented: BTreeMap<SpuVectorComponent, ReferenceOmission>,
}

impl SpuVectorComparison {
    /// Whether every compared component matched its expected value, and
    /// every open one matched CellGov's documented choice.
    pub fn is_match(&self) -> bool {
        self.differences.is_empty() && self.unchosen.is_empty()
    }

    /// Records one component: a difference when no value matches, an
    /// unchosen result when a legal value other than the documented
    /// choice matches.
    fn check<T>(
        &mut self,
        component: SpuVectorComponent,
        field: &SpuField<T>,
        matches: impl Fn(&T) -> bool,
    ) {
        match field {
            SpuField::Value { value } => {
                self.compared.insert(component);
                if !matches(value) {
                    self.differences.insert(component);
                }
            }
            SpuField::OneOf { values, chosen, .. } => {
                self.compared.insert(component);
                if !values.iter().any(&matches) {
                    self.differences.insert(component);
                } else if !values.get(*chosen).is_some_and(&matches) {
                    self.unchosen.insert(component);
                }
            }
            SpuField::Undefined { .. } => {
                self.unrepresented
                    .insert(component, ReferenceOmission::Undefined);
            }
            SpuField::Unsupported { .. } => {
                self.unrepresented
                    .insert(component, ReferenceOmission::Unsupported);
            }
        }
    }
}

/// What one vector's replay observed.
#[derive(Debug, Clone, Copy)]
pub(super) struct SpuVectorObservation<'a> {
    /// The state the SPU started in, words loaded.
    pub loaded: &'a SpuState,
    /// The SPU's context at the end.
    pub snapshot: &'a SpuSnapshot,
    /// How the replay ended.
    pub end: SpuReferenceEnd,
    /// Every effect, in emission order.
    pub effects: &'a [SpuReferenceEffect],
    /// Each main-storage region's base and bytes at the start.
    pub initial_memory: &'a [(u64, Vec<u8>)],
    /// Each main-storage region's base and bytes at the end.
    pub memory: &'a [(u64, Vec<u8>)],
    /// The other SPU's local store at the start.
    pub initial_peer_ls: Option<&'a [u8]>,
    /// The other SPU's state and inbound mailbox at the end.
    pub peer: Option<(&'a SpuState, &'a [u32])>,
    /// One result per problem-state operation.
    pub ppu_results: &'a [SpuReferencePpuResult],
    /// Commands the MFC refused.
    pub mfc_exceptions: &'a [SpuReferenceInvalidCommand],
}

/// Compares every component of `observed` the expected end names.
///
/// [Martignoni2009 p:127 s:2.2] The compared state is the program counter, the registers, the memory, and the exception. After an exception the other three stay as they were.
pub(super) fn compare_vector(
    expected: &SpuReferenceFinal,
    observed: &SpuVectorObservation<'_>,
) -> SpuVectorComparison {
    let state = &observed.snapshot.state;
    let mut comparison = SpuVectorComparison::default();
    comparison.check(SpuVectorComponent::End, &expected.end, |end| {
        *end == observed.end
    });
    let regs = |overrides: &BTreeMap<String, String>| {
        let mut bank = *observed.loaded.regs.as_array();
        for (index, hex) in overrides {
            match (parse_index(index, bank.len()), parse_register(hex)) {
                (Some(index), Some(value)) => bank[index] = value,
                _ => return false,
            }
        }
        bank == *state.regs.as_array()
    };
    comparison.check(SpuVectorComponent::Registers, &expected.regs_hex, regs);
    comparison.check(
        SpuVectorComponent::LocalStore,
        &expected.local_store,
        |runs| {
            let mut ls = observed.loaded.ls.clone();
            write_runs(&mut ls, 0, runs);
            ls == state.ls
        },
    );
    comparison.check(SpuVectorComponent::ProgramCounter, &expected.pc, |pc| {
        *pc == state.pc
    });
    comparison.check(SpuVectorComponent::Lslr, &expected.lslr, |lslr| {
        *lslr == state.lslr()
    });
    comparison.check(SpuVectorComponent::Fpscr, &expected.fpscr, |hex| {
        parse_fpscr(hex) == Some(state.fpscr())
    });
    comparison.check(SpuVectorComponent::Stop, &expected.stop, |stop| {
        *stop == state.stop.as_ref().map(SpuReferenceStop::from)
    });
    comparison.check(
        SpuVectorComponent::InterruptsEnabled,
        &expected.interrupts_enabled,
        |enabled| *enabled == state.interrupts_enabled(),
    );
    comparison.check(SpuVectorComponent::Srr0, &expected.srr0, |srr0| {
        *srr0 == state.srr0()
    });
    comparison.check(SpuVectorComponent::Signals, &expected.signals, |signals| {
        *signals == state.signals.each_ref().map(SpuReferenceSignal::from)
    });
    comparison.check(
        SpuVectorComponent::Channels,
        &expected.channels,
        |channels| *channels == SpuReferenceChannelState::from(&state.channels),
    );
    comparison.check(
        SpuVectorComponent::Reservation,
        &expected.reservation,
        |line| *line == state.reservation().map(|line| line.addr()),
    );
    comparison.check(SpuVectorComponent::Effects, &expected.effects, |effects| {
        effects.as_slice() == observed.effects
    });
    comparison.check(
        SpuVectorComponent::MainMemory,
        &expected.main_memory,
        |runs| {
            let mut image = observed.initial_memory.to_vec();
            for (base, bytes) in &mut image {
                write_runs(bytes, *base, runs);
            }
            image == observed.memory
        },
    );
    comparison.check(SpuVectorComponent::Peer, &expected.peer, |peer| {
        let (Some(initial), Some((state, inbox))) = (observed.initial_peer_ls, observed.peer)
        else {
            return false;
        };
        let mut ls = initial.to_vec();
        write_runs(&mut ls, 0, &peer.local_store);
        ls == state.ls
            && peer.signals == state.signals.each_ref().map(SpuReferenceSignal::from)
            && peer.in_mbox == inbox
    });
    comparison.check(
        SpuVectorComponent::PpuResults,
        &expected.ppu_results,
        |results| results.as_slice() == observed.ppu_results,
    );
    comparison.check(
        SpuVectorComponent::MfcExceptions,
        &expected.mfc_exceptions,
        |exceptions| exceptions.as_slice() == observed.mfc_exceptions,
    );
    comparison
}
