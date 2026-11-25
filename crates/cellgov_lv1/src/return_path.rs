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
//! problem-state caller. The `hrfid` values here assume that two
//! branches of the return are not taken:
//! - the call that delivers a pending event to the partition
//!   [OFW-4.93 lv1 a:0x1010-0x101C];
//! - the call that sets the decrementer to 0
//!   [OFW-4.93 lv1 a:0x104C-0x1050].
//!
//! CellGov's hypervisor gives no event and no decrementer request, so
//! neither branch runs.

use cellgov_ps3_abi::lv1::dispatch::{CALLER_TSRL, HRFID_KEEPS_CR7_FIRMWARE};
use cellgov_ps3_abi::lv1::table::HYPERCALL_TABLE_ENTRIES;

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

/// The registers that the return through `hrfid` gives a caller in
/// problem state when `problem_state` is true, from a hypervisor for the
/// firmware version `firmware`.
///
/// The return gives these values:
/// - r0: [`CALLER_TSRL`].
/// - r11 and r12: the saved SRR0 and SRR1 [OFW-4.93 lv1 a:0x1038-0x103C].
/// - CR0: `EQ`, from the compare of the decrementer request with 0
///   [OFW-4.93 lv1 a:0x104C].
/// - CR1: the signed compare of `MSR[PR]` with 1 [OFW-4.93 lv1 a:0x0F9C].
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
