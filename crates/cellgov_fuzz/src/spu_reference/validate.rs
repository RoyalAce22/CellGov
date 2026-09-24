//! Parsing and validation of an SPU reference artifact, and its initial state.

use cellgov_spu::state::{SpuState, SPU_LS_SIZE, SPU_REG_COUNT};

use crate::reference::is_lower_hex;

use super::types::{
    SpuReferenceArtifact, SpuReferenceError, SpuReferenceInput, SPU_REFERENCE_SCHEMA_VERSION,
};

/// Parses and validates a bounded repository reference artifact.
// [Jiang2022 p:4 s:3] Cases come from the machine-readable specification, and a separate engine compares them on real devices and emulators.
pub fn parse_reference_json(json: &str) -> Result<SpuReferenceArtifact, SpuReferenceError> {
    let artifact: SpuReferenceArtifact = serde_json::from_str(json)?;
    artifact.validate()?;
    Ok(artifact)
}

pub(super) fn parse_register(hex: &str) -> Option<[u8; 16]> {
    if !is_lower_hex(hex, 32) {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut result = [0u8; 16];
    for (index, byte) in result.iter_mut().enumerate() {
        let high = (bytes[index * 2] as char).to_digit(16)? as u8;
        let low = (bytes[index * 2 + 1] as char).to_digit(16)? as u8;
        *byte = high << 4 | low;
    }
    Some(result)
}

pub(super) fn parse_index(key: &str, limit: usize) -> Option<usize> {
    let index = key.parse::<usize>().ok()?;
    (index < limit && index.to_string() == key).then_some(index)
}

impl SpuReferenceInput {
    pub(super) fn to_state(&self) -> Result<SpuState, SpuReferenceError> {
        let mut state = SpuState::new();
        state.pc = self.pc;
        for (index, hex) in &self.regs_hex {
            let index = parse_index(index, SPU_REG_COUNT).ok_or(SpuReferenceError::Invalid {
                field: "initial_state.regs_hex",
            })?;
            state.regs[index] = parse_register(hex).ok_or(SpuReferenceError::Invalid {
                field: "initial_state.regs_hex",
            })?;
        }
        for (offset, &byte) in &self.local_store {
            let offset = parse_index(offset, SPU_LS_SIZE).ok_or(SpuReferenceError::Invalid {
                field: "initial_state.local_store",
            })?;
            state.ls[offset] = byte;
        }
        if let Some(channels) = &self.channels {
            state.channels.mfc_lsa = channels.mfc_lsa;
            state.channels.mfc_eah = channels.mfc_eah;
            state.channels.mfc_eal = channels.mfc_eal;
            state.channels.mfc_size = channels.mfc_size;
            state.channels.mfc_tag_id = channels.mfc_tag_id;
            state.channels.tag_mask = channels.tag_mask;
            state.channels.tag_status = channels.tag_status;
            state.channels.atomic_status = channels.atomic_status;
            state.channels.pending_mbox_rt = channels.pending_mbox_rt;
            state.channels.pending_get = channels.pending_get;
        }
        state.reservation = self.reservation.map(cellgov_sync::ReservedLine::containing);
        Ok(state)
    }
}

impl SpuReferenceArtifact {
    pub(super) fn validate(&self) -> Result<(), SpuReferenceError> {
        let invalid = |field| SpuReferenceError::Invalid { field };
        if self.schema_version != SPU_REFERENCE_SCHEMA_VERSION {
            return Err(SpuReferenceError::Version {
                found: self.schema_version,
                supported: SPU_REFERENCE_SCHEMA_VERSION,
            });
        }
        if self.case_id.trim().is_empty() {
            return Err(invalid("case_id"));
        }
        if self.words.is_empty()
            || self.words.len() > 64
            || self.initial_state.pc & 3 != 0
            || (self.initial_state.pc as usize)
                .checked_add(self.words.len() * 4)
                .is_none_or(|end| end > SPU_LS_SIZE)
        {
            return Err(invalid("words"));
        }
        for (field, regs) in [
            ("initial_state.regs_hex", Some(&self.initial_state.regs_hex)),
            ("expected.regs_hex", self.expected.regs_hex.as_value()),
        ] {
            if regs.is_some_and(|regs| {
                regs.iter().any(|(index, hex)| {
                    parse_index(index, SPU_REG_COUNT).is_none() || parse_register(hex).is_none()
                })
            }) {
                return Err(invalid(field));
            }
        }
        if self
            .initial_state
            .local_store
            .keys()
            .any(|offset| parse_index(offset, SPU_LS_SIZE).is_none())
        {
            return Err(invalid("initial_state.local_store"));
        }
        if self.expected.local_store.as_value().is_some_and(|ls| {
            ls.keys()
                .any(|offset| parse_index(offset, SPU_LS_SIZE).is_none())
        }) {
            return Err(invalid("expected.local_store"));
        }
        if self
            .initial_state
            .reservation
            .is_some_and(|addr| cellgov_sync::ReservedLine::containing(addr).addr() != addr)
        {
            return Err(invalid("initial_state.reservation"));
        }
        if self
            .expected
            .reservation
            .as_value()
            .is_some_and(|reservation| {
                reservation
                    .is_some_and(|addr| cellgov_sync::ReservedLine::containing(addr).addr() != addr)
            })
        {
            return Err(invalid("expected.reservation"));
        }
        if self
            .initial_state
            .channels
            .as_ref()
            .is_some_and(|channels| {
                channels
                    .pending_mbox_rt
                    .is_some_and(|register| register as usize >= SPU_REG_COUNT)
            })
        {
            return Err(invalid("initial_state.channels.pending_mbox_rt"));
        }
        if self.expected.channels.as_value().is_some_and(|channels| {
            channels
                .pending_mbox_rt
                .is_some_and(|register| register as usize >= SPU_REG_COUNT)
        }) {
            return Err(invalid("expected.channels.pending_mbox_rt"));
        }
        if self
            .expected
            .effects
            .as_value()
            .is_some_and(|effects| !effects.is_empty())
        {
            return Err(invalid("expected.effects"));
        }
        for (field, blank) in [
            ("expected.regs_hex", self.expected.regs_hex.blank_reason()),
            (
                "expected.local_store",
                self.expected.local_store.blank_reason(),
            ),
            ("expected.pc", self.expected.pc.blank_reason()),
            ("expected.channels", self.expected.channels.blank_reason()),
            (
                "expected.reservation",
                self.expected.reservation.blank_reason(),
            ),
            ("expected.outcome", self.expected.outcome.blank_reason()),
            ("expected.effects", self.expected.effects.blank_reason()),
            (
                "expected.fault_discarded",
                self.expected.fault_discarded.blank_reason(),
            ),
        ] {
            if blank.is_some() {
                return Err(invalid(field));
            }
        }
        self.provenance
            .check(valid_spu_citation)
            .map_err(|_| invalid("provenance"))
    }
}

pub(super) fn valid_spu_citation(citation: &str) -> bool {
    let Some((page, section)) = citation
        .strip_prefix("SPU-ISA p:")
        .and_then(|remainder| remainder.split_once(" s:"))
    else {
        return false;
    };
    let Ok(number) = page.parse::<u16>() else {
        return false;
    };
    number > 0
        && number.to_string() == page
        && !section.trim().is_empty()
        && section.trim() == section
}
