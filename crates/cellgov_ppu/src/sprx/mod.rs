//! Parser and loader for decrypted PS3 firmware PRX (ELF64 type 0xFFA4).
//!
//! Game-side import parsing lives in [`crate::prx`].

mod exports;
mod load;
mod parse;
mod phdr;
mod relocated_pointer;
#[cfg(test)]
#[path = "tests/test_fixtures.rs"]
pub(crate) mod test_fixtures;

pub use load::{
    load_prx, LoadedOpd, LoadedPrx, PrxLoadError, RelocMisalignedKind, R_PPC64_ADDR16_HA,
    R_PPC64_ADDR16_HI, R_PPC64_ADDR16_LO, R_PPC64_ADDR16_LO_DS, R_PPC64_ADDR32, R_PPC64_ADDR64,
    R_PPC64_REL24,
};
pub use parse::{
    module_identity, parse_prx, ParsedPrx, PrxExport, PrxExportLib, PrxOpd, PrxParseError,
    PrxRelocation, PrxSegment,
};
pub(crate) use relocated_pointer::{relocated_pointer_image, RelocatedPointerError};
