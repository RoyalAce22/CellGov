//! PUP (PlayStation Update Package) container facts.
//!
//! Entry ids are a fixed table Sony assigns to the payloads a firmware
//! update carries; a reader locates a payload by id, never by position.

/// `update_files.tar`: the TAR of SCE-wrapped dev_flash packages that
/// carries the firmware image itself.
///
/// RPCS3 reads the same id in `rpcs3qt/main_window.cpp`
/// `HandlePupInstallation`.
pub const ENTRY_ID_UPDATE_FILES: u64 = 0x300;
