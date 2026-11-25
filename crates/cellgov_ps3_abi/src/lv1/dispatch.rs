//! What lv1's own `sc` (LEV=1) interrupt vector returns in r3 before it
//! reaches a hypercall handler:
//! - the value for a number past the end of the hypercall table;
//! - the value for a problem-state call that the table entry does not
//!   allow.
//!
//! These values have no public source. They were read from the
//! firmware's own `lv1.self`, decrypted offline and disassembled with
//! `cellgov dev disasm`. No byte of lv1 is committed; these facts are.
//!
//! The vector is at the system-call interrupt address 0x0c00. Its entry
//! block, 0x0c00 to 0x0c58, is byte-identical in the lv1 images of 3.15,
//! 3.41, 3.70 and 4.93. The 1.94 block differs only in the targets of
//! its branches at 0x0c20 and 0x0c40. The 1.50 block also reads the
//! table's count and base at other offsets from HSPRG0. The dispatch
//! block after it differs by version. At 3.15 and later each version
//! loads its own TOC address into r2 at 0x0fa4 and 0x0fac: 3.15
//! 0x350470, 3.41 0x34E0E0, 3.70 0x35A548 and 4.93 0x35A038.
//!
//! The vector returns each of its own statuses zero-extended: a `li r3`
//! of the status, then `clrldi r3, r3, 32`. So r3 holds the status in
//! its low word and 0 in its high word.

/// The r3 that lv1 returns for a hypercall number past the end of its
/// hypercall table: `LV1_NOT_IMPLEMENTED` (-20), zero-extended.
///
/// The vector compares the number in r11 against the table's entry count
/// (`cmpld r11, r0` at 0x0c34). For a number that is not less than the
/// count, it runs `li r3, -20` at 0x0c44 and `clrldi r3, r3, 32` at
/// 0x0c50, then `rfid`. The instructions at 0x0c34, 0x0c44 and 0x0c50
/// are identical in the lv1 images of 1.50, 1.94, 3.15, 3.41, 3.70 and
/// 4.93.
///
/// The range check runs before the problem-state check. So a number past
/// the end gets this value from either processor state. This path
/// changes no register from r4 to r10.
pub const UNKNOWN_HYPERCALL_R3: u64 = 0x0000_0000_FFFF_FFEC;

/// The r3 that lv1 returns for a problem-state `sc 1` to a number in the
/// table whose entry is not problem-state callable: `LV1_NO_PRIVILEGE`
/// (-3), zero-extended.
///
/// lv1 checks `SRR1[PR]` on entry. The vector reads SRR1 (`mfspr r12,
/// 27`) and extracts `MSR[PR]`, SRR1 bit 49 (`rldicl r12, r12, 50, 63`).
/// When the caller is in problem state and the table entry's low bit is
/// clear, the vector runs `li r3, -3` and `clrldi r3, r3, 32`. A
/// problem-state call is answered, never stopped.
///
/// Addresses by firmware:
/// - 3.15, 3.41, 3.70, 4.93: SRR1 read 0x0f64, PR extract 0x0f90,
///   status 0x1088, zero-extension 0x108c.
/// - 1.50, 1.94: dispatch at 0x09b0; SRR1 read 0x09b4, PR extract
///   0x09e0, status 0x0ad0.
pub const PROBLEM_STATE_DENIED_R3: u64 = 0x0000_0000_FFFF_FFFD;

#[cfg(test)]
#[path = "tests/dispatch_tests.rs"]
mod tests;
