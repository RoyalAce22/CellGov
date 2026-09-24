//! Title-tree layout facts: where a disc or HDD title keeps its files.

/// The directory a disc holds its game files under, relative to the
/// disc root. An HDD title's tree has no such level: its game files
/// sit at the tree root.
pub const DISC_GAME_DIR: &str = "PS3_GAME";

/// The system-software update a retail disc carries, relative to the
/// disc root; the console installs it when its own firmware is older.
pub const DISC_UPDATE_PUP: &str = "PS3_UPDATE/PS3UPDAT.PUP";

/// The directory a title tree holds its executable under: at the root
/// of an HDD title's tree, and under [`DISC_GAME_DIR`] on a disc.
pub const USRDIR: &str = "USRDIR";

/// The disc drive's mount, as a bare directory name.
pub const BDVD_MOUNT: &str = "dev_bdvd";

/// The path the guest addresses the disc drive at.
pub const GUEST_BDVD: &str = "/dev_bdvd";

#[cfg(test)]
#[path = "tests/title_tree_tests.rs"]
mod tests;
