//! SPU instruction execution: translates decoded instructions into
//! `SpuState` mutations and `Effect` packets.

mod branch;
mod channel;
mod compare;
mod constant;
mod dispatch;
mod double;
mod estimate;
mod float;
mod integer;
mod lanes;
mod logical;
mod ls;
mod outcome;
mod shift;
mod slots;

pub(crate) use channel::{invalid_command, mfc_barrier_kind, start_held_lists};
pub use dispatch::execute;
pub(crate) use float::to_integer_can_refuse;
pub use outcome::{SpuFault, SpuStepOutcome};
