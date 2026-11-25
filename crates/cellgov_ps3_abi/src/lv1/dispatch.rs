//! What lv1's own `sc` (LEV=1) interrupt vector returns:
//! - the r3 for a number past the end of the hypercall table;
//! - the r3 for a problem-state call that the table entry does not
//!   allow;
//! - the r0 of every return;
//! - the CR7 of the return through `hrfid`, by firmware.
//!
//! These values have no public source. They were read from the
//! firmware's own `lv1.self`, decrypted offline and disassembled with
//! `cellgov dev disasm`. No byte of lv1 is committed; these facts are.
//!
//! The vector is at the system-call interrupt address 0x0c00. Its entry
//! block is byte-identical in the lv1 images of 3.15
//! [OFW-3.15 lv1 a:0x0c00-0x0c58], 3.41 [OFW-3.41 lv1 a:0x0c00-0x0c58],
//! 3.70 [OFW-3.70 lv1 a:0x0c00-0x0c58] and 4.93
//! [OFW-4.93 lv1 a:0x0c00-0x0c58]. The 1.94 block differs only in the
//! targets of its branches [OFW-1.94 lv1 a:0x0c20] [OFW-1.94 lv1 a:0x0c40].
//! The 1.50 block also reads the table's count and base at other offsets
//! from HSPRG0 [OFW-1.50 lv1 a:0x0c24-0x0c30]. The dispatch block after it
//! differs by version. At 3.15 and later each version loads its own TOC
//! address into r2: 3.15 0x350470 [OFW-3.15 lv1 a:0x0fa4-0x0fac], 3.41
//! 0x34E0E0 [OFW-3.41 lv1 a:0x0fa4-0x0fac], 3.70 0x35A548
//! [OFW-3.70 lv1 a:0x0fa4-0x0fac] and 4.93 0x35A038
//! [OFW-4.93 lv1 a:0x0fa4-0x0fac].
//!
//! The vector returns each of its own statuses zero-extended: a `li r3`
//! of the status, then `clrldi r3, r3, 32`. So r3 holds the status in
//! its low word and 0 in its high word.

/// The r3 that lv1 returns for a hypercall number past the end of its
/// hypercall table: `LV1_NOT_IMPLEMENTED` (-20), zero-extended.
///
/// The vector compares the number in r11 against the table's entry count
/// (`cmpld r11, r0` [OFW-4.93 lv1 a:0x0c34]). For a number that is not
/// less than the count, it runs `li r3, -20` [OFW-4.93 lv1 a:0x0c44] and
/// `clrldi r3, r3, 32` [OFW-4.93 lv1 a:0x0c50], then `rfid`. These
/// instructions are identical in the lv1 images of 1.50
/// [OFW-1.50 lv1 a:0x0c34-0x0c50], 1.94 [OFW-1.94 lv1 a:0x0c34-0x0c50],
/// 3.15 [OFW-3.15 lv1 a:0x0c34-0x0c50], 3.41
/// [OFW-3.41 lv1 a:0x0c34-0x0c50] and 3.70 [OFW-3.70 lv1 a:0x0c34-0x0c50].
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
/// - 4.93: SRR1 read [OFW-4.93 lv1 a:0x0f64], PR extract
///   [OFW-4.93 lv1 a:0x0f90], status [OFW-4.93 lv1 a:0x1088],
///   zero-extension [OFW-4.93 lv1 a:0x108c]. The 3.15
///   [OFW-3.15 lv1 a:0x0f64-0x108c], 3.41 [OFW-3.41 lv1 a:0x0f64-0x108c]
///   and 3.70 [OFW-3.70 lv1 a:0x0f64-0x108c] blocks hold them at the same
///   addresses.
/// - 1.50: dispatch [OFW-1.50 lv1 a:0x09b0]; SRR1 read
///   [OFW-1.50 lv1 a:0x09b4], PR extract [OFW-1.50 lv1 a:0x09e0], status
///   [OFW-1.50 lv1 a:0x0ad0]. The 1.94 block
///   [OFW-1.94 lv1 a:0x09b0-0x0ad0] holds them at the same addresses.
pub const PROBLEM_STATE_DENIED_R3: u64 = 0x0000_0000_FFFF_FFFD;

/// The r0 that lv1 returns from every hypercall: the caller's TSRL.
///
/// The `lv1_sc_return_registers` capture shows this value after each of
/// its ten calls.
/// The vector saves SPR 896 at entry [OFW-4.93 lv1 a:0x0c0c-0x0c10]. Each
/// return loads the saved value into r0 and writes it back to SPR 896:
/// - the past-the-table return [OFW-4.93 lv1 a:0x0c48-0x0c54];
/// - the return through `hrfid` [OFW-4.93 lv1 a:0x1054-0x1078].
///
/// The value holds TP = 0b10 (medium) and FWDP = 0x4000. CellGov does not
/// model TSRL, so it gives every caller this value.
///
/// [CBE-Handbook p:307 s:10.4.5] TSRL bits 11:12 hold TP, the thread priority, with 0b10 for medium, and bits 44:63 hold FWDP, the forward-progress timer.
pub const CALLER_TSRL: u64 = 0x0010_0000_0000_4000;

/// The firmware versions whose vector, on the return through `hrfid`,
/// calls a function that leaves `GT` in CR7.
///
/// The return calls it with r3 = 1 [OFW-4.93 lv1 a:0x1020-0x1024]. Each
/// path through the function ends with a compare of a positive value
/// with 0 in CR7 [OFW-4.93 lv1 a:0x1240-0x132c]. The 3.15
/// [OFW-3.15 lv1 a:0x0f60-0x107c], 3.41 [OFW-3.41 lv1 a:0x0f60-0x107c] and
/// 3.70 [OFW-3.70 lv1 a:0x0f60-0x107c] returns hold the same instructions
/// at the same addresses, except the TOC address that each loads into r2.
/// Their functions hold the same instructions at the same addresses:
/// 3.15 [OFW-3.15 lv1 a:0x1240-0x132c], 3.41
/// [OFW-3.41 lv1 a:0x1240-0x132c] and 3.70 [OFW-3.70 lv1 a:0x1240-0x132c].
pub const HRFID_CR7_FIRMWARE: &[&str] = &["3.15", "3.41", "3.70", "4.93"];

/// The firmware versions whose return through `hrfid` does not call
/// the function of [`HRFID_CR7_FIRMWARE`], and so leaves CR7 as the
/// handler left it.
///
/// The 1.50 [OFW-1.50 lv1 a:0x09b0-0x0ac4] and 1.94
/// [OFW-1.94 lv1 a:0x09b0-0x0ac4] returns hold the other compares of the
/// 4.93 return, and no such call.
pub const HRFID_KEEPS_CR7_FIRMWARE: &[&str] = &["1.50", "1.94"];

#[cfg(test)]
#[path = "tests/dispatch_tests.rs"]
mod tests;
