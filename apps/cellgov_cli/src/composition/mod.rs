//! The boot command's face on store-driven composition.
//!
//! `cellgov_boot::compose` resolves `--fw` and `--game-ver` against the
//! store's [`StoreInventory`] and builds the guest tree the pair
//! produces. This module keeps what only the command owns:
//!
//! - [`refusal`] words each typed refusal around the flags;
//! - [`banner`] prints the choice before any other output.
//!
//! [`StoreInventory`]: cellgov_install::store::StoreInventory

pub(crate) mod banner;
pub(crate) mod refusal;

pub(crate) use cellgov_boot::compose::{
    compose_boot, BootComposition, ComposeInputs, FirmwareChoice, GameChoice,
};
pub(crate) use cellgov_install::store::select::{
    FirmwareSelectError, GameVersion, GameVersionSelectError,
};
pub(crate) use refusal::ComposeError;
