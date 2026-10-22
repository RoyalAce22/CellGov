//! SPU architectural state (registers, LS, PC, limit register, signal
//! registers, channels, reservation, stopped state).

mod channel;
mod events;
mod signal;
mod snapshot;
mod unit;

pub use cellgov_ps3_abi::hw::spu::{SPU_LSLR_FULL, SPU_LS_SIZE, SPU_REG_COUNT};
pub use channel::{ChannelState, ListCursor, TagUpdateCondition};
pub use signal::{SignalNotifyMode, SignalNotifyRegister};
pub use snapshot::{
    SpuChannelSnapshot, SpuEventSnapshot, SpuFacilitySnapshot, SpuListSnapshot,
    SpuObservableSnapshot,
};
pub use unit::{RegBank, SpuState, STATE_HASH_SCHEME};
