//! Store-driven boot composition: which firmware and which title
//! version a boot runs against, and the guest-visible tree that pair
//! produces.
//!
//! - [`inventory`] reads the store's install records.
//! - [`select`] resolves `--fw` and `--game-ver` against that inventory
//!   to one choice or one refusal.
//! - [`compose`] builds the ordered host roots the mount table and the
//!   EBOOT probe use.
//! - [`identity`] names the choice in the form every machine artifact
//!   the boot writes embeds.
//! - [`banner`] prints the choice before any other output.

pub(crate) mod banner;
pub(crate) mod compose;
pub(crate) mod identity;
pub(crate) mod inventory;
pub(crate) mod select;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

pub(crate) use compose::{
    compose_boot, BootComposition, ComposeError, ComposeInputs, ComposedMount, GameChoice,
};
pub(crate) use select::{FirmwareChoice, FirmwareSelectError, GameVersion, GameVersionSelectError};
