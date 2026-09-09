//! Title-tree layout facts: where a disc or HDD title keeps its files.

/// The directory a disc holds its game files under, relative to the
/// disc root. An HDD title's tree has no such level: its game files
/// sit at the tree root.
pub const DISC_GAME_DIR: &str = "PS3_GAME";
