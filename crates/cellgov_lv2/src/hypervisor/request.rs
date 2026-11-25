//! The hypercall a supervisor thread yielded.

use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

/// One hypercall: its number, its input registers and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HypercallRequest {
    /// The unit that executed `sc`.
    pub unit: UnitId,
    /// The hypercall number, from r11.
    pub number: u64,
    /// r3 to r10, in that order.
    pub inputs: [u64; 8],
    /// The address of the `sc`.
    pub pc: u64,
    /// The PPE hardware thread the unit ran on, when the unit names one.
    pub thread: Option<u8>,
    /// The guest time of the call.
    pub tick: GuestTicks,
}

impl HypercallRequest {
    /// The value of input register `reg`, one of r3 to r10.
    ///
    /// # Panics
    ///
    /// Panics when `reg` is outside 3 to 10.
    pub fn r(&self, reg: usize) -> u64 {
        self.inputs[reg - 3]
    }
}
