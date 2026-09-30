//! SPU instruction execution: translates decoded instructions into
//! `SpuState` mutations and `Effect` packets.

mod channel;
mod dispatch;
mod double;
mod estimate;
mod float;
mod lanes;
mod ls;
mod outcome;

pub use dispatch::execute;
pub(crate) use float::scale;
pub use outcome::{SpuFault, SpuStepOutcome};
