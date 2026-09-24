//! SPU instruction execution: translates decoded instructions into
//! `SpuState` mutations and `Effect` packets.

mod channel;
mod dispatch;
mod ls;
mod outcome;

pub use dispatch::execute;
pub use outcome::{SpuFault, SpuStepOutcome};
