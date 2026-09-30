//! Parses and validates an SPU reference vector set.

use std::collections::BTreeSet;

use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;
use cellgov_ps3_abi::hw::spu::{
    channel_direction, MFC_SPU_QUEUE_DEPTH, SPU_IN_MBOX_DEPTH, SPU_LSLR_FULL, SPU_LS_SIZE,
    SPU_REG_COUNT, SPU_STOP_CODE_MASK,
};
use cellgov_ps3_abi::hw::spu_isa::SPU_OPCODE_MAP;
use cellgov_ps3_abi::hw::spu_mfc::{mfc_opcode_class, MfcOpcodeClass, MfcQueues};
use cellgov_ps3_abi::lv2::spu::thread_window;
use serde::Deserialize;

use super::set_convert::parse_hex_bytes;
use super::set_types::{
    SpuField, SpuReferenceBytes, SpuReferenceChannelState, SpuReferenceEffect, SpuReferenceFile,
    SpuReferencePpuAction, SpuReferenceSet, SpuReferenceStart, SpuReferenceStop, SpuReferenceUnit,
    SpuReferenceVector, SpuReferenceWorld, MAX_SPU_REFERENCE_STEPS, MAX_SPU_REFERENCE_VECTORS,
    MAX_SPU_REFERENCE_WORDS, MAX_SPU_REFERENCE_WORLD_BYTES, SPU_REFERENCE_SET_SCHEMA_VERSION,
};
use super::types::{SpuReferenceError, SPU_REFERENCE_SCHEMA_VERSION};
use super::validate::{
    parse_fpscr, parse_index, parse_reference_json, parse_register, valid_spu_citation,
};

/// Parses and validates a vector set.
pub fn parse_reference_set_json(json: &str) -> Result<SpuReferenceSet, SpuReferenceError> {
    let set: SpuReferenceSet = serde_json::from_str(json)?;
    set.validate()?;
    Ok(set)
}

/// Parses a reference file of either form, chosen by its schema version.
pub fn parse_reference_file(json: &str) -> Result<SpuReferenceFile, SpuReferenceError> {
    #[derive(Deserialize)]
    struct Version {
        schema_version: u32,
    }
    let Version { schema_version } = serde_json::from_str(json)?;
    match schema_version {
        SPU_REFERENCE_SCHEMA_VERSION => {
            parse_reference_json(json).map(|artifact| SpuReferenceFile::Single(Box::new(artifact)))
        }
        SPU_REFERENCE_SET_SCHEMA_VERSION => {
            parse_reference_set_json(json).map(SpuReferenceFile::Set)
        }
        found => Err(SpuReferenceError::Version {
            found,
            supported: SPU_REFERENCE_SET_SCHEMA_VERSION,
        }),
    }
}

fn invalid(field: &'static str) -> SpuReferenceError {
    SpuReferenceError::Invalid { field }
}

/// Whether `check` holds, or the field's refusal.
fn require(check: bool, field: &'static str) -> Result<(), SpuReferenceError> {
    if check {
        Ok(())
    } else {
        Err(invalid(field))
    }
}

/// Whether every run parses and lies below `limit`, and no two runs
/// overlap.
pub(super) fn runs_fit(runs: &[SpuReferenceBytes], limit: u64) -> bool {
    let mut spans = Vec::with_capacity(runs.len());
    for run in runs {
        let Some(bytes) = parse_hex_bytes(&run.hex) else {
            return false;
        };
        match run.at.checked_add(bytes.len() as u64) {
            Some(end) if end <= limit => spans.push((run.at, end)),
            _ => return false,
        }
    }
    spans.sort_unstable();
    spans.windows(2).all(|pair| pair[0].1 <= pair[1].0)
}

/// Whether every run of `runs` lies inside one run of `regions`.
pub(super) fn runs_inside(runs: &[SpuReferenceBytes], regions: &[SpuReferenceBytes]) -> bool {
    runs.iter().all(|run| {
        let Some(len) = parse_hex_bytes(&run.hex).map(|bytes| bytes.len() as u64) else {
            return false;
        };
        regions.iter().any(|region| {
            let region_len = region.hex.len() as u64 / 2;
            run.at >= region.at
                && run
                    .at
                    .checked_add(len)
                    .is_some_and(|end| end <= region.at + region_len)
        })
    })
}

fn regs_ok(regs: &std::collections::BTreeMap<String, String>) -> bool {
    regs.iter().all(|(index, hex)| {
        parse_index(index, SPU_REG_COUNT).is_some() && parse_register(hex).is_some()
    })
}

fn ls_address_ok(address: u32) -> bool {
    address & 3 == 0 && (address as usize) < SPU_LS_SIZE
}

fn stop_ok(stop: &SpuReferenceStop) -> bool {
    u32::from(stop.code) <= SPU_STOP_CODE_MASK && ls_address_ok(stop.npc)
}

/// Whether a channel state converts and keeps its queues inside their
/// depths.
fn channels_ok(channels: &SpuReferenceChannelState) -> bool {
    channels.to_channels().is_some()
        && channels.in_mbox.len() <= SPU_IN_MBOX_DEPTH as usize
        && channels.mfc_cmd_count <= MFC_SPU_QUEUE_DEPTH
        && channels.lists.len() <= MFC_SPU_QUEUE_DEPTH as usize
}

fn line_ok(addr: u64) -> bool {
    cellgov_sync::ReservedLine::containing(addr).addr() == addr
}

/// Whether the field is well formed: a blank reason refuses, and a set
/// of legal values holds two or more distinct values, a chosen index
/// inside them, and values each `ok` accepts.
fn field_ok<T: PartialEq>(field: &SpuField<T>, ok: impl Fn(&T) -> bool) -> bool {
    match field {
        SpuField::Value { value } => ok(value),
        SpuField::OneOf {
            values,
            chosen,
            reason,
        } => {
            values.len() >= 2
                && *chosen < values.len()
                && !reason.trim().is_empty()
                && values.iter().all(&ok)
                && values
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !values[..index].contains(value))
        }
        SpuField::Undefined { reason } | SpuField::Unsupported { reason } => {
            !reason.trim().is_empty()
        }
    }
}

impl SpuReferenceSet {
    pub(super) fn validate(&self) -> Result<(), SpuReferenceError> {
        if self.schema_version != SPU_REFERENCE_SET_SCHEMA_VERSION {
            return Err(SpuReferenceError::Version {
                found: self.schema_version,
                supported: SPU_REFERENCE_SET_SCHEMA_VERSION,
            });
        }
        self.unit.validate()?;
        require(
            !self.vectors.is_empty() && self.vectors.len() <= MAX_SPU_REFERENCE_VECTORS,
            "vectors",
        )?;
        let mut names = BTreeSet::new();
        for vector in &self.vectors {
            require(
                !vector.name.trim().is_empty() && names.insert(vector.name.as_str()),
                "vectors.name",
            )?;
        }
        for (index, vector) in self.vectors.iter().enumerate() {
            vector
                .validate()
                .map_err(|source| SpuReferenceError::InVector {
                    index,
                    source: Box::new(source),
                })?;
        }
        Ok(())
    }
}

impl SpuReferenceUnit {
    fn validate(&self) -> Result<(), SpuReferenceError> {
        match self {
            Self::Instruction { mnemonic } => require(
                SPU_OPCODE_MAP.iter().any(|row| row.mnemonic == mnemonic),
                "unit.mnemonic",
            ),
            Self::Channel { number } => {
                require(channel_direction(*number).is_some(), "unit.number")
            }
            Self::MfcCommand { opcode } => require(
                matches!(
                    mfc_opcode_class(*opcode),
                    MfcOpcodeClass::Defined(def) if def.queues != MfcQueues::ProxyOnly
                ),
                "unit.opcode",
            ),
            Self::UnassignedOpcodes
            | Self::ReservedChannels
            | Self::OutsideSpuQueue
            | Self::Facility { .. } => Ok(()),
        }
    }
}

impl SpuReferenceVector {
    fn validate(&self) -> Result<(), SpuReferenceError> {
        self.provenance
            .check(valid_spu_citation)
            .map_err(|_| invalid("provenance"))?;
        let start = &self.initial_state;
        require(
            !self.words.is_empty()
                && self.words.len() <= MAX_SPU_REFERENCE_WORDS
                && start.pc & 3 == 0
                && (start.pc as usize)
                    .checked_add(self.words.len() * 4)
                    .is_some_and(|end| end <= SPU_LS_SIZE),
            "words",
        )?;
        require(
            (1..=MAX_SPU_REFERENCE_STEPS).contains(&self.step_limit),
            "step_limit",
        )?;
        start.validate()?;
        self.world.validate(self.step_limit)?;
        self.validate_expected()
    }

    fn validate_expected(&self) -> Result<(), SpuReferenceError> {
        let expected = &self.expected;
        let world = &self.world;
        require(field_ok(&expected.end, |_| true), "expected.end")?;
        require(field_ok(&expected.regs_hex, regs_ok), "expected.regs_hex")?;
        require(
            field_ok(&expected.local_store, |runs| {
                runs_fit(runs, SPU_LS_SIZE as u64)
            }),
            "expected.local_store",
        )?;
        require(
            field_ok(&expected.pc, |pc| ls_address_ok(*pc)),
            "expected.pc",
        )?;
        require(
            field_ok(&expected.lslr, |lslr| lslr & !SPU_LSLR_FULL == 0),
            "expected.lslr",
        )?;
        require(
            field_ok(&expected.fpscr, |hex| parse_fpscr(hex).is_some()),
            "expected.fpscr",
        )?;
        require(
            field_ok(&expected.stop, |stop| stop.as_ref().is_none_or(stop_ok)),
            "expected.stop",
        )?;
        require(
            field_ok(&expected.interrupts_enabled, |_| true),
            "expected.interrupts_enabled",
        )?;
        require(
            field_ok(&expected.srr0, |srr0| ls_address_ok(*srr0)),
            "expected.srr0",
        )?;
        require(field_ok(&expected.signals, |_| true), "expected.signals")?;
        require(
            field_ok(&expected.channels, channels_ok),
            "expected.channels",
        )?;
        require(
            field_ok(&expected.reservation, |line| line.is_none_or(line_ok)),
            "expected.reservation",
        )?;
        require(
            field_ok(&expected.effects, |effects| effects.iter().all(effect_ok)),
            "expected.effects",
        )?;
        require(
            field_ok(&expected.main_memory, |runs| {
                runs_fit(runs, u64::MAX) && runs_inside(runs, &world.memory)
            }),
            "expected.main_memory",
        )?;
        require(
            field_ok(&expected.peer, |peer| {
                world.peer.is_some()
                    && runs_fit(&peer.local_store, SPU_LS_SIZE as u64)
                    && peer.in_mbox.len() <= SPU_IN_MBOX_DEPTH as usize
            }),
            "expected.peer",
        )?;
        require(
            field_ok(&expected.ppu_results, |results| {
                results.len() == world.ppu.len()
            }),
            "expected.ppu_results",
        )?;
        require(
            field_ok(&expected.mfc_exceptions, |_| true),
            "expected.mfc_exceptions",
        )
    }
}

fn effect_ok(effect: &SpuReferenceEffect) -> bool {
    match effect {
        SpuReferenceEffect::SharedWrite { hex, .. }
        | SpuReferenceEffect::ConditionalStore { hex, .. } => parse_hex_bytes(hex).is_some(),
        SpuReferenceEffect::Dma { tag, payload, .. } => {
            tag.is_none_or(|tag| tag < 32)
                && payload
                    .as_deref()
                    .is_none_or(|hex| parse_hex_bytes(hex).is_some())
        }
        SpuReferenceEffect::SharedRead { .. }
        | SpuReferenceEffect::ReservationAcquire { .. }
        | SpuReferenceEffect::MailboxPop { .. }
        | SpuReferenceEffect::InvalidCommand { .. } => true,
        SpuReferenceEffect::Other { effect } => !effect.trim().is_empty(),
    }
}

impl SpuReferenceStart {
    fn validate(&self) -> Result<(), SpuReferenceError> {
        require(regs_ok(&self.regs_hex), "initial_state.regs_hex")?;
        require(
            runs_fit(&self.local_store, SPU_LS_SIZE as u64),
            "initial_state.local_store",
        )?;
        require(
            self.lslr.is_none_or(|lslr| lslr & !SPU_LSLR_FULL == 0),
            "initial_state.lslr",
        )?;
        require(
            self.fpscr
                .as_deref()
                .is_none_or(|hex| parse_fpscr(hex).is_some()),
            "initial_state.fpscr",
        )?;
        require(self.stop.as_ref().is_none_or(stop_ok), "initial_state.stop")?;
        require(ls_address_ok(self.srr0), "initial_state.srr0")?;
        require(
            self.channels.as_ref().is_none_or(channels_ok),
            "initial_state.channels",
        )?;
        require(
            self.reservation.is_none_or(line_ok),
            "initial_state.reservation",
        )
    }
}

impl SpuReferenceWorld {
    fn validate(&self, step_limit: u32) -> Result<(), SpuReferenceError> {
        // The SPU thread window takes the top 256 MB of the 32-bit space,
        // so no main-storage region may sit there.
        let window = thread_window::BASE..=u64::from(u32::MAX);
        require(
            runs_fit(&self.memory, CELL_EA_LIMIT + 1)
                && self.memory.iter().all(|run| {
                    let end = run.at + run.hex.len() as u64 / 2;
                    end <= *window.start() || run.at > *window.end()
                })
                && self
                    .memory
                    .iter()
                    .map(|run| run.hex.len() as u64 / 2)
                    .sum::<u64>()
                    <= MAX_SPU_REFERENCE_WORLD_BYTES,
            "world.memory",
        )?;
        require(
            self.peer
                .as_ref()
                .is_none_or(|peer| runs_fit(&peer.local_store, SPU_LS_SIZE as u64)),
            "world.peer",
        )?;
        require(
            self.ppu.windows(2).all(|pair| pair[0].step <= pair[1].step)
                && self.ppu.iter().all(|write| {
                    write.step < step_limit
                        && match write.action {
                            SpuReferencePpuAction::Signal { register, .. }
                            | SpuReferencePpuAction::SignalMode { register, .. } => {
                                register == 1 || register == 2
                            }
                            SpuReferencePpuAction::InMbox { .. }
                            | SpuReferencePpuAction::ReadOutMbox
                            | SpuReferencePpuAction::StopRequest
                            | SpuReferencePpuAction::WriteNpc { .. }
                            | SpuReferencePpuAction::Restart => true,
                        }
                }),
            "world.ppu",
        )
    }
}
