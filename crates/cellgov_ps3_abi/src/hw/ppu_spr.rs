//! The problem-state class of each PPE SPR number for `mfspr` and `mtspr`.
//!
//! The rows are the SPRs that the documents define for the PPE. A
//! direction that the figures omit for a defined number takes its class
//! from spr0:
//!
//! - spr0 = 1: privileged.
//! - spr0 = 0: reserved.
//!
//! Every other SPR number is reserved. For a reserved number with
//! spr0 = 1, CBEA E.4 gives a privileged interrupt and the PPE handbook
//! gives an illegal one. The table follows the handbook. A problem-state
//! program cannot tell the two apart.
//!
//! No available document gives the numbers of the PPE implementation
//! registers (HID0-6, TSCR, TTR and others). The table holds them as
//! reserved, although the PPE can answer privileged for some of them.
//! The optional External Control and Performance Monitor facilities of
//! Book III are not PPE SPRs, so EAR and SPRs 768-799 are reserved too.
// [PPC-Book3 p:19 s:3.4.2] Figure 17 lists the mtspr encodings; spr0=1 iff writing is privileged; an optional SPR the implementation lacks is as if not listed.
// [PPC-Book3 p:21 s:3.4.2] Figure 18 lists the mfspr encodings; spr0=1 iff reading is privileged; every SPR number not listed is reserved.
// [PPC-Book2 p:30 s:4.1] mftb names the time base as TBR 268 and its upper half as TBR 269; the v2.02 mfspr figure does not list them.
// The reference console runs mfspr 268 and 269 in problem state and reads the time base (probes ppu_may_trap_mfspr268 and ppu_may_trap_mfspr269).
// [AltiVec-PEM p:2-6 s:2.3.3] VRSAVE is SPR 256, a user register.
// [CBEA p:303 s:Appendix C] Table C-1 places the CBEA PPE SPRs: BP_VR, the RMT, range and class-ID registers.
// [CBEA p:304 s:Appendix C] Table C-1 places the TLB management SPRs.
// [CBEA p:312 s:Appendix E] E.4: a problem-state access to an inaccessible SPR takes a privileged or illegal interrupt by the high-order bit of the SPR number.
// [CBEA p:313 s:Appendix E] E.4 names reads of write-only SPRs among the inaccessible accesses.
// [CBE-Handbook p:254 s:9.5.9] The PPE takes a privileged-instruction interrupt for a problem-state access to a privileged SPR, and an illegal-instruction interrupt for a reserved SPR.

/// What the PPE does with a problem-state `mfspr` or `mtspr` of an SPR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SprClass {
    /// The access runs.
    User,
    /// The PPE takes a privileged-instruction interrupt.
    Privileged,
    /// The PPE takes an illegal-instruction interrupt.
    Reserved,
}

/// One SPR the documents define, with its class for each direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SprRow {
    /// The SPR number, with its two 5-bit halves in reading order.
    pub spr: u16,
    /// The register's name.
    pub name: &'static str,
    /// The class of `mfspr` of this number.
    pub read: SprClass,
    /// The class of `mtspr` of this number.
    pub write: SprClass,
}

const fn row(spr: u16, name: &'static str, read: SprClass, write: SprClass) -> SprRow {
    SprRow {
        spr,
        name,
        read,
        write,
    }
}

use SprClass::{Privileged as P, Reserved as R, User as U};

/// The SPRs the documents define for the PPE, sorted by number.
pub const PPU_SPR_MAP: &[SprRow] = &[
    row(1, "XER", U, U),
    row(8, "LR", U, U),
    row(9, "CTR", U, U),
    row(18, "DSISR", P, P),
    row(19, "DAR", P, P),
    row(22, "DEC", P, P),
    row(25, "SDR1", P, P),
    row(26, "SRR0", P, P),
    row(27, "SRR1", P, P),
    row(29, "ACCR", P, P),
    row(136, "CTRL", U, R),
    row(152, "CTRL", P, P),
    row(256, "VRSAVE", U, U),
    row(259, "SPRG3", U, R),
    row(268, "TB", U, R),
    row(269, "TBU", U, R),
    row(272, "SPRG0", P, P),
    row(273, "SPRG1", P, P),
    row(274, "SPRG2", P, P),
    row(275, "SPRG3", P, P),
    row(284, "TBL", P, P),
    row(285, "TBU", P, P),
    row(287, "PVR", P, P),
    row(304, "HSPRG0", P, P),
    row(305, "HSPRG1", P, P),
    row(309, "PURR", P, P),
    row(310, "HDEC", P, P),
    row(312, "RMOR", P, P),
    row(313, "HRMOR", P, P),
    row(314, "HSRR0", P, P),
    row(315, "HSRR1", P, P),
    row(318, "LPCR", P, P),
    row(319, "LPIDR", P, P),
    row(946, "PPE_TLB_Index_Hint", P, P),
    row(947, "PPE_TLB_Index", P, P),
    row(948, "PPE_TLB_RPN", P, P),
    row(949, "PPE_TLB_VPN", P, P),
    row(950, "PPE_TLB_RMT_Index", P, P),
    row(951, "PPE_TLB_RMT_Data", P, P),
    row(952, "DRSR0", P, P),
    row(953, "DRMR0", P, P),
    row(954, "DCIDR0", P, P),
    row(955, "DRSR1", P, P),
    row(956, "DRMR1", P, P),
    row(957, "DCIDR1", P, P),
    row(976, "IRSR0", P, P),
    row(977, "IRMR0", P, P),
    row(978, "ICIDR0", P, P),
    row(979, "IRSR1", P, P),
    row(980, "IRMR1", P, P),
    row(981, "ICIDR1", P, P),
    row(1013, "DABR", P, P),
    row(1015, "DABRX", P, P),
    row(1022, "BP_VR", P, P),
    row(1023, "PIR", P, P),
];

/// The row for SPR `spr`, or `None` for a reserved number.
pub const fn spr_row(spr: u16) -> Option<&'static SprRow> {
    let mut i = 0;
    while i < PPU_SPR_MAP.len() {
        if PPU_SPR_MAP[i].spr == spr {
            return Some(&PPU_SPR_MAP[i]);
        }
        i += 1;
    }
    None
}

/// The class of a problem-state `mfspr` of SPR `spr`.
pub const fn spr_read_class(spr: u16) -> SprClass {
    match spr_row(spr) {
        Some(row) => row.read,
        None => SprClass::Reserved,
    }
}

/// The class of a problem-state `mtspr` of SPR `spr`.
pub const fn spr_write_class(spr: u16) -> SprClass {
    match spr_row(spr) {
        Some(row) => row.write,
        None => SprClass::Reserved,
    }
}

/// CTRL CT for a read on hardware thread 0: bits 0:1 = 0b10.
///
/// [CBE-Handbook p:307 s:10.4.4] CTRL bits 0:1 (CT) name the reading thread, 8:9 (TE) enable the threads, 31 is RUN.
pub const CTRL_CT_THREAD0: u32 = 0x8000_0000;

/// CTRL CT for a read on hardware thread 1: bits 0:1 = 0b01.
///
/// [CBE-Handbook p:307 s:10.4.4] See [`CTRL_CT_THREAD0`].
pub const CTRL_CT_THREAD1: u32 = 0x4000_0000;

/// CTRL TE bit 8: hardware thread 0 is enabled.
///
/// [CBE-Handbook p:307 s:10.4.4] See [`CTRL_CT_THREAD0`].
pub const CTRL_TE_THREAD0: u32 = 0x0080_0000;

/// CTRL TE bit 9: hardware thread 1 is enabled.
///
/// [CBE-Handbook p:307 s:10.4.4] See [`CTRL_CT_THREAD0`].
pub const CTRL_TE_THREAD1: u32 = 0x0040_0000;

/// The CTRL value a PS3 process's only running thread reads: thread 1
/// reads it, and only thread 1 is enabled.
///
/// The reference console reads it from `main()` in both runs of the
/// probe `ppu_may_trap_mfspr136`. In the `ppu_ctrl_threads` capture,
/// three concurrent threads read all four combinations of CT and TE.
pub const PS3_PROCESS_CTRL: u32 = CTRL_CT_THREAD1 | CTRL_TE_THREAD1;

/// The XER bits an `mtxer` keeps: SO, OV and CA (bits 32:34) and the
/// byte count (bits 57:63).
///
/// [PPC-Book1 p:32 s:3.2.2] XER bits 0:31 and 35:56 are reserved.
///
/// [PPC-Book1 p:3 s:1.5.2] A reserved bit written with 1 reads back as 0 or 1, by implementation.
///
/// The reference console reads the reserved bits back as 0 after an
/// `mtxer` of all ones (probe `ppu_reference_vectors`).
pub const XER_IMPLEMENTED_BITS: u64 = !super::ppu_status::XER_RESERVED;

/// The SPRG3 value a problem-state PS3 process reads.
///
/// [PPC-Book3 p:16 s:3.3.3] Problem-state software may read SPRG3; its content is the operating system's.
///
/// The reference console reads 0 in both runs of the probe
/// `ppu_may_trap_mfspr259`.
pub const PS3_PROCESS_SPRG3: u64 = 0;

#[cfg(test)]
#[path = "tests/ppu_spr_tests.rs"]
mod tests;
