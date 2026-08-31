//! PUP (PlayStation Update Package) container facts.
//!
//! Entry ids are a fixed table Sony assigns to the payloads a firmware
//! update carries; a reader locates a payload by id, never by position.

/// `update_files.tar`: the TAR of SCE-wrapped dev_flash packages that
/// carries the firmware image itself.
///
/// Every retail update package carries this payload under this id.
pub const ENTRY_ID_UPDATE_FILES: u64 = 0x300;
