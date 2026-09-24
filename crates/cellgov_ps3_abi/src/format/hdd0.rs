//! Internal hard disk (`dev_hdd0`) layout facts: where installed games
//! and the user profile's license files live.

/// The internal hard disk's mount, as a bare directory name.
pub const HDD0_MOUNT: &str = "dev_hdd0";

/// The directory under [`HDD0_MOUNT`] that holds one tree per installed
/// game, each named by its title id.
pub const GAME_DIR: &str = "game";

/// The path the guest addresses the installed-game directory at.
pub const GUEST_GAME_DIR: &str = "/dev_hdd0/game";

/// The directory under [`HDD0_MOUNT`] that holds the user profiles.
pub const HOME_DIR: &str = "home";

/// The one user profile CellGov models, as its directory name under
/// [`HOME_DIR`].
pub const USER_DIR: &str = "00000001";

/// The directory under a user profile that holds its license files.
pub const EXDATA_DIR: &str = "exdata";

/// The path the guest addresses the modeled user's license directory
/// at.
pub const GUEST_EXDATA_DIR: &str = "/dev_hdd0/home/00000001/exdata";

#[cfg(test)]
#[path = "tests/hdd0_tests.rs"]
mod tests;
