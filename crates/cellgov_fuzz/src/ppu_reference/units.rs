//! The architectural units a PPU reference file covers, listed from the
//! documents, and the unit each decoded kind belongs to.
//!
//! An instruction unit is one mnemonic, and its record and OE forms
//! share it. Three group units hold the words with no instruction unit:
//!
//! - the instructions the PPE does not implement;
//! - the Book III instructions;
//! - the words no instruction selects.
//!
//! A facility unit covers a behaviour no single instruction exercises.

use serde::{Deserialize, Serialize};

use cellgov_ppu::instruction::fuzz::{generation_descriptor, PpuFuzzKind};
use cellgov_ppu::instruction::PpuInstructionKind as K;
use cellgov_ps3_abi::hw::ppc_mnemonics::{
    PPC_VECTOR_INSTRUCTIONS, PPE_BOOK_I_II_INSTRUCTIONS, PPE_NEW_INSTRUCTIONS,
    PPE_VECTOR_INSTRUCTIONS,
};

/// The architectural unit a PPU reference file covers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PpuReferenceUnit {
    /// One mnemonic of the documented instruction listings.
    Instruction {
        /// The mnemonic.
        mnemonic: String,
    },
    /// The instructions the PPE does not implement.
    PpeUnimplemented,
    /// The Book III instructions, privileged in problem state.
    Privileged,
    /// The words no instruction selects.
    UnassignedOpcodes,
    /// A behaviour no single instruction exercises.
    Facility {
        /// Which facility.
        name: PpuReferenceFacility,
    },
}

/// A PPU behaviour with a reference file of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PpuReferenceFacility {
    /// The state a new thread starts in.
    ThreadEntry,
    /// The reservation: its granule, its loss, and a conditional store to
    /// another address.
    Reservation,
    /// The size of the block `dcbz` zeroes.
    DcbzWidth,
    /// The time base.
    TimeBase,
    /// A load of bytes a store before it wrote.
    StoreForwarding,
    /// A string load or store across a page boundary.
    StringAcrossPage,
    /// The FPSCR's own rules: sticky exception bits, the FX, FEX and VX
    /// summaries, and the record forms' copy into CR1.
    Fpscr,
}

impl PpuReferenceFacility {
    /// Every facility, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::ThreadEntry,
        Self::Reservation,
        Self::DcbzWidth,
        Self::TimeBase,
        Self::StoreForwarding,
        Self::StringAcrossPage,
        Self::Fpscr,
    ];

    /// The name a reference file writes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::ThreadEntry => "thread_entry",
            Self::Reservation => "reservation",
            Self::DcbzWidth => "dcbz_width",
            Self::TimeBase => "time_base",
            Self::StoreForwarding => "store_forwarding",
            Self::StringAcrossPage => "string_across_page",
            Self::Fpscr => "fpscr",
        }
    }
}

/// The key a unit has in the pending list and the completeness report.
pub fn unit_key(unit: &PpuReferenceUnit) -> String {
    match unit {
        PpuReferenceUnit::Instruction { mnemonic } => format!("instruction:{mnemonic}"),
        PpuReferenceUnit::PpeUnimplemented => "ppe_unimplemented".into(),
        PpuReferenceUnit::Privileged => "privileged".into(),
        PpuReferenceUnit::UnassignedOpcodes => "unassigned_opcodes".into(),
        PpuReferenceUnit::Facility { name } => format!("facility:{}", name.name()),
    }
}

/// Every unit a reference file can cover, in listing order: the
/// instructions, then the three group units, then the facilities.
pub fn all_units() -> Vec<PpuReferenceUnit> {
    let instructions = PPE_BOOK_I_II_INSTRUCTIONS
        .iter()
        .chain(PPE_NEW_INSTRUCTIONS)
        .chain(PPC_VECTOR_INSTRUCTIONS)
        .chain(PPE_VECTOR_INSTRUCTIONS)
        .map(|mnemonic| PpuReferenceUnit::Instruction {
            mnemonic: (*mnemonic).into(),
        });
    let groups = [
        PpuReferenceUnit::PpeUnimplemented,
        PpuReferenceUnit::Privileged,
        PpuReferenceUnit::UnassignedOpcodes,
    ];
    let facilities = PpuReferenceFacility::ALL
        .into_iter()
        .map(|name| PpuReferenceUnit::Facility { name });
    instructions.chain(groups).chain(facilities).collect()
}

/// The unit a decoded kind belongs to.
pub fn kind_unit(kind: PpuFuzzKind) -> PpuReferenceUnit {
    let instruction = |mnemonic: &str| PpuReferenceUnit::Instruction {
        mnemonic: mnemonic.into(),
    };
    let named = |name: String| instruction(&name.to_ascii_lowercase());
    match kind {
        PpuFuzzKind::Vx(op) => named(format!("{op:?}")),
        PpuFuzzKind::Va(op) => named(format!("{op:?}")),
        PpuFuzzKind::Fp59(op) => named(format!("{op:?}")),
        PpuFuzzKind::Fp63(op) => named(format!("{op:?}")),
        PpuFuzzKind::Ordinary(kind) => {
            ordinary_unit(kind).unwrap_or_else(|| named(format!("{kind:?}")))
        }
    }
}

/// The unit an instruction word belongs to, or `None` for a word with no
/// generation recipe.
pub fn word_unit(raw: u32) -> Option<PpuReferenceUnit> {
    let kind = generation_descriptor(raw)?.kind;
    // [AltiVec-PEM p:A-1 s:A.1] dssall, dstt and dststt are mnemonics of
    // their own; word bit 6, the A or T field, selects each.
    let bit_6 = (raw >> 25) & 1 == 1;
    let mnemonic = match kind {
        PpuFuzzKind::Ordinary(K::Dss) if bit_6 => "dssall",
        PpuFuzzKind::Ordinary(K::Dst) if bit_6 => "dstt",
        PpuFuzzKind::Ordinary(K::Dstst) if bit_6 => "dststt",
        _ => return Some(kind_unit(kind)),
    };
    Some(PpuReferenceUnit::Instruction {
        mnemonic: mnemonic.into(),
    })
}

/// The unit of a kind whose name is not its mnemonic.
fn ordinary_unit(kind: K) -> Option<PpuReferenceUnit> {
    let instruction = |mnemonic: &str| {
        Some(PpuReferenceUnit::Instruction {
            mnemonic: mnemonic.into(),
        })
    };
    match kind {
        K::AddicDot => instruction("addic."),
        K::AndiDot => instruction("andi."),
        K::AndisDot => instruction("andis."),
        K::Stwcx => instruction("stwcx."),
        K::Stdcx => instruction("stdcx."),
        K::Cmpw | K::Cmpd => instruction("cmp"),
        K::Cmpwi | K::Cmpdi => instruction("cmpi"),
        K::Cmplw | K::Cmpld => instruction("cmpl"),
        K::Cmplwi | K::Cmpldi => instruction("cmpli"),
        K::Mftbu => instruction("mftb"),
        K::Mflr | K::Mfctr | K::Mfxer | K::Mfvrsave | K::Mfctrl | K::Mfsprg3 => {
            instruction("mfspr")
        }
        K::Mtlr | K::Mtctr | K::Mtxer | K::Mtvrsave => instruction("mtspr"),
        K::Popcntb | K::Mcrxr | K::PpeIllegal => Some(PpuReferenceUnit::PpeUnimplemented),
        K::PpePrivileged => Some(PpuReferenceUnit::Privileged),
        _ => None,
    }
}
