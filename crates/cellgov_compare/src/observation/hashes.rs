//! CellGov-side state hashes and the serde bridge for `StateHash`.

use cellgov_trace::StateHash;
use serde::{Deserialize, Serialize};

mod state_hash_serde {
    use cellgov_trace::StateHash;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(hash: &StateHash, s: S) -> Result<S::Ok, S::Error> {
        hash.raw().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<StateHash, D::Error> {
        u64::deserialize(d).map(StateHash::new)
    }
}

/// CellGov-side state hashes for replay comparison (CellGov-vs-CellGov).
///
/// The RPCS3 adapter sets this to `None`; cross-runner comparison does
/// not use these hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedHashes {
    /// Hash of committed guest memory.
    #[serde(with = "state_hash_serde")]
    pub memory: StateHash,
    /// Hash of all unit status values.
    #[serde(with = "state_hash_serde")]
    pub unit_status: StateHash,
    /// Hash of sync primitive state.
    #[serde(with = "state_hash_serde")]
    pub sync: StateHash,
    /// Scheme id of the three hashes above. Deserialization supplies
    /// [`CHECKPOINT_HASH_SCHEME`] when an observation omits the field.
    #[serde(default = "checkpoint_hash_scheme")]
    pub scheme: u64,
}

/// Scheme id of the three commit-checkpoint hashes: FNV-1a over a tag.
///
/// The PPU per-step hash is not one of the three, so a change to the
/// PPU scheme leaves this id as it is. When a change to one of the three
/// producers changes a hash value, increase the tag's version suffix.
pub const CHECKPOINT_HASH_SCHEME: u64 = {
    let mut h = cellgov_mem::Fnv1aHasher::new();
    h.write(b"cellgov-checkpoint-fnv1a/v1");
    h.finish()
};

/// The scheme of an observation that names none.
fn checkpoint_hash_scheme() -> u64 {
    CHECKPOINT_HASH_SCHEME
}
