//! Parser and loader for decrypted PS3 firmware PRX (ELF64 type 0xFFA4).
//!
//! Game-side import parsing lives in [`crate::prx`].

mod load;
mod parse;
#[cfg(test)]
pub(crate) mod test_fixtures;

pub use load::{
    is_applier_supported, load_prx, LoadedOpd, LoadedPrx, PrxLoadError, RelocMisalignedKind,
    APPLIER_SUPPORTED_TYPES, R_PPC64_ADDR16_HA, R_PPC64_ADDR16_HI, R_PPC64_ADDR16_LO,
    R_PPC64_ADDR16_LO_DS, R_PPC64_ADDR32, R_PPC64_ADDR64, R_PPC64_REL24,
};
pub use parse::{
    parse_prx, ParsedPrx, PrxExport, PrxExportLib, PrxOpd, PrxParseError, PrxRelocation, PrxSegment,
};
