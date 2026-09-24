//! Store-driven boot composition: which firmware and which title
//! version a boot runs against, the guest-visible tree that pair
//! produces, and the identity triple every artifact of the boot embeds.
//!
//! [`compose_boot`] resolves both selections through
//! `cellgov_install::store::select`, then builds the ordered host roots
//! the mount table and the executable probe use. The refusals are typed
//! and name no command-line flag; a caller that takes the selections
//! from flags words the refusal around them.

mod composition;
mod identity;

pub use composition::{
    compose_boot, BootComposition, ComposeError, ComposeInputs, FirmwareChoice, GameChoice,
    StoredGame, UnderstatedFirmware,
};
pub use identity::{
    tree_app_version, FirmwareClaims, FirmwareIdentityError, GameIdentityError, IdentityError,
};
