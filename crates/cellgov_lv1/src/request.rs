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
    /// The link register at the `sc`. In a leaf function it is the
    /// return address into the function's caller.
    pub lr: u64,
    /// The PPE hardware thread the unit ran on, when the unit names one.
    pub thread: Option<u8>,
    /// Whether the caller was in problem state, `MSR[PR]` = 1.
    pub problem_state: bool,
    /// The guest time of the call.
    pub tick: GuestTicks,
}

/// Whether the unit that executed `sc` takes the page table load of an
/// address space selection.
///
/// lv1 loads SDR1 and HID6\[LB\] on the calling thread as part of the
/// selection, so the selection and the load succeed or fail together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerTranslation {
    /// The unit holds SDR1 and HID6 and takes the load.
    Loads,
    /// The unit holds no supervisor registers and refuses the load.
    Refuses,
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
