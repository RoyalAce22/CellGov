//! What lv1's two returns from its `sc` vector leave in r0, r11, r12 and
//! CR, beside r3 to r10.
//!
//! The vector returns one of two ways:
//! - A number past the end of the table returns through `rfid`
//!   [OFW-4.93 lv1 a:0x0C44-0x0C58].
//! - Every number in the table returns through `hrfid`
//!   [OFW-4.93 lv1 a:0x1038-0x107C], after its handler or the vector's
//!   problem-state refusal.
//!
//! The `lv1_sc_return_registers` capture shows both returns from a
//! problem-state caller. CellGov's hypervisor gives no decrementer
//! request, so the `hrfid` values here assume that the return does not
//! call the function that sets the decrementer to 0
//! [OFW-4.93 lv1 a:0x104C-0x1050].
//!
//! The `hrfid` return raises the caller's external interrupt when the
//! caller's status shadow and mask share a bit
//! [OFW-4.93 lv1 a:0x1010-0x101C]; see
//! [`cellgov_ps3_abi::lv1::interrupt_mask`]. [`external_delivery`] gives
//! what the raise does, and [`delivering_table`] the registers that the
//! return then leaves.

use cellgov_ps3_abi::hw::ppu_interrupt::SRR1_INTERRUPT_INFO;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_EE, MSR_INTERRUPT_CLEARED};
use cellgov_ps3_abi::lv1::dispatch::{CALLER_TSRL, HRFID_KEEPS_CR7_FIRMWARE};
use cellgov_ps3_abi::lv1::interrupt::THREADS_PER_LOGICAL_PPE;
use cellgov_ps3_abi::lv1::interrupt_mask::{
    external_interrupt_msr, EXTERNAL_INTERRUPT_MSR_CLEARED, EXTERNAL_INTERRUPT_VECTOR,
};
use cellgov_ps3_abi::lv1::table::HYPERCALL_TABLE_ENTRIES;

use crate::answer::UnmodelledError;

/// The MSR that the `sc` of each PPE hardware thread saved, by thread, or
/// `None` for a thread whose unit gives none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SavedMsrs(pub [Option<u64>; THREADS_PER_LOGICAL_PPE as usize]);

impl SavedMsrs {
    /// The saved MSR of the thread `thread`, or `None`.
    pub fn of(&self, thread: u8) -> Option<u64> {
        self.0.get(usize::from(thread)).copied().flatten()
    }
}

/// The external interrupt that lv1's return raises for a thread: the
/// return enters the vector with the new MSR.
///
/// The thread's SRR0 is the address after its `sc`, and its SRR1 is its
/// saved MSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalDelivery {
    /// The vector, [`EXTERNAL_INTERRUPT_VECTOR`].
    pub vector: u64,
    /// The MSR at the vector, [`external_interrupt_msr`] of the saved MSR.
    pub msr: u64,
}

/// What lv1's raise does for a thread whose `sc` saved the MSR `msr`.
///
/// # Errors
///
/// Returns the refusal text in these cases:
/// - `MSR[EE]` is 0. lv1 then sets LPCR\[MER\]
///   [OFW-4.93 lv1 a:0x191C-0x1928], and a later return from an
///   interrupt raises the external interrupt under conditions whose
///   meaning no image read gives [OFW-4.93 lv1 a:0x1880-0x18A8].
/// - The MSR holds one of bits 48 to 63 that the PPU's interrupt entry
///   keeps, such as ME. lv1 clears each of these bits.
/// - The MSR holds one of bits 33 to 36 or 42 to 47. lv1 gives SRR1 the
///   whole MSR, and the PPU's interrupt entry clears these bits.
pub(crate) fn external_delivery(msr: u64) -> Result<ExternalDelivery, UnmodelledError> {
    if msr & MSR_EE == 0 {
        return Err(UnmodelledError::new(
            "an external interrupt on the return to a thread whose saved MSR[EE] is 0, for which \
             lv1 sets LPCR[MER]",
        ));
    }
    if msr & EXTERNAL_INTERRUPT_MSR_CLEARED & !MSR_INTERRUPT_CLEARED != 0 {
        return Err(UnmodelledError::new(
            "an external interrupt on the return to a thread whose saved MSR holds a bit of 48 to \
             63 that lv1 clears and the PPU's interrupt entry keeps",
        ));
    }
    if msr & SRR1_INTERRUPT_INFO != 0 {
        return Err(UnmodelledError::new(
            "an external interrupt on the return to a thread whose saved MSR holds a bit that lv1 \
             gives SRR1 and the PPU's interrupt entry clears",
        ));
    }
    Ok(ExternalDelivery {
        vector: EXTERNAL_INTERRUPT_VECTOR,
        msr: external_interrupt_msr(msr),
    })
}

/// The result that a compare in lv1's vector puts in a CR field. The
/// field's SO bit is the caller's XER\[SO\].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compare {
    /// LT.
    Less,
    /// GT.
    Greater,
    /// EQ.
    Equal,
}

/// Where lv1's return takes r11 and r12 from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnPair {
    /// These values.
    Values {
        /// The value for r11.
        r11: u64,
        /// The value for r12.
        r12: u64,
    },
    /// The SRR0 and SRR1 that the caller's `sc` set: the address after
    /// the `sc`, and the caller's MSR.
    SaveRestore,
}

/// What one of lv1's returns leaves in r0, r11, r12 and CR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnRegisters {
    /// The value for r0.
    pub r0: u64,
    /// Where r11 and r12 come from.
    pub r11_r12: ReturnPair,
    /// The compare result in each CR field, CR0 first, or `None` for a
    /// field that keeps the caller's value.
    pub cr: [Option<Compare>; 8],
}

/// The registers that the past-the-table return gives for hypercall
/// `number`, from a table at `table_base`.
///
/// The return gives these values:
/// - r0: [`CALLER_TSRL`].
/// - r11: `number` shifted left by 3 [OFW-4.93 lv1 a:0x0C3C].
/// - r12: the table base [OFW-4.93 lv1 a:0x0C30].
/// - CR0: the unsigned compare of `number` with the entry count
///   [OFW-4.93 lv1 a:0x0C34]. Every other CR field keeps its value.
pub(crate) fn past_the_table(number: u64, table_base: u32) -> ReturnRegisters {
    let mut cr = [None; 8];
    cr[0] = Some(unsigned_compare(number, HYPERCALL_TABLE_ENTRIES));
    ReturnRegisters {
        r0: CALLER_TSRL,
        r11_r12: ReturnPair::Values {
            r11: number << 3,
            r12: u64::from(table_base),
        },
        cr,
    }
}

/// The registers that the return through `hrfid` gives the caller, from
/// a hypervisor for the firmware version `firmware`.
///
/// The return gives these values:
/// - r0: [`CALLER_TSRL`].
/// - r11 and r12: the saved SRR0 and SRR1 [OFW-4.93 lv1 a:0x1038-0x103C].
/// - CR0: `EQ`, from the compare of the decrementer request with 0
///   [OFW-4.93 lv1 a:0x104C].
/// - CR1: the signed compare of `MSR[PR]` with 1, where `problem_state`
///   gives `MSR[PR]` [OFW-4.93 lv1 a:0x0F9C].
/// - CR7: `GT`, except at a firmware of [`HRFID_KEEPS_CR7_FIRMWARE`].
///
/// A handler can also change CR1, CR5 and CR6, and CellGov does not
/// model those changes. At a firmware whose vector no reading gives, the
/// return is the 4.93 return.
pub(crate) fn table(problem_state: bool, firmware: Option<&str>) -> ReturnRegisters {
    let mut cr = [None; 8];
    cr[0] = Some(Compare::Equal);
    cr[1] = Some(if problem_state {
        Compare::Equal
    } else {
        Compare::Less
    });
    if !firmware.is_some_and(|fw| HRFID_KEEPS_CR7_FIRMWARE.contains(&fw)) {
        cr[7] = Some(Compare::Greater);
    }
    ReturnRegisters {
        r0: CALLER_TSRL,
        r11_r12: ReturnPair::SaveRestore,
        cr,
    }
}

/// The registers that the return through `hrfid` gives a caller whose
/// external interrupt lv1 raises with `delivery`.
///
/// The registers are those of [`table`], except r11 and r12. lv1 loads
/// them from the saved pc and MSR [OFW-4.93 lv1 a:0x1038-0x103C] after the
/// raise replaced those two values [OFW-4.93 lv1 a:0x190C-0x1914]. So r11
/// is the vector and r12 the new MSR.
pub(crate) fn delivering_table(
    problem_state: bool,
    firmware: Option<&str>,
    delivery: ExternalDelivery,
) -> ReturnRegisters {
    ReturnRegisters {
        r11_r12: ReturnPair::Values {
            r11: delivery.vector,
            r12: delivery.msr,
        },
        ..table(problem_state, firmware)
    }
}

fn unsigned_compare(a: u64, b: u64) -> Compare {
    match a.cmp(&b) {
        std::cmp::Ordering::Less => Compare::Less,
        std::cmp::Ordering::Greater => Compare::Greater,
        std::cmp::Ordering::Equal => Compare::Equal,
    }
}

#[cfg(test)]
#[path = "tests/return_path_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/console_sc_return_registers_tests.rs"]
mod console_tests;
