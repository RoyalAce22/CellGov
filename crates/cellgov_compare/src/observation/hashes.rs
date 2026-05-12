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
}
