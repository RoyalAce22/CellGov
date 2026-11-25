//! The supervisor registers of one PPE hardware thread.

use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_PR, PPE_SLB_ENTRIES, SLB_VALID};

use crate::multilinear::{supervisor_lane, SUPERVISOR_WORDS};

const _: () = assert!(SUPERVISOR_WORDS == 11 + 2 * PPE_SLB_ENTRIES);

/// One SLB entry: the effective half `slbmte` takes from RB and the
/// virtual half it takes from RS.
///
/// [PPC-Book3 p:51 s:4.11.3.1] Figure 32: RB holds the ESID and V, RS the VSID and Ks, Kp, N, L and C.
/// [CBE-Handbook p:84 s:4.2.5.2] The PPE adds LP in RS bit 59 and does not implement VSID bits 0:14.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlbEntry {
    /// The ESID in bits 0:35 and V in bit 36; every other bit 0.
    pub esid_v: u64,
    /// VSID bits 15:51, Ks, Kp, N, L and C in bits 52:56, and LP in bit
    /// 59; every other bit 0.
    pub vsid_flags: u64,
}

impl SlbEntry {
    /// Whether V is 1.
    pub fn valid(&self) -> bool {
        self.esid_v & SLB_VALID != 0
    }
}

/// The registers a thread in supervisor state reads and writes.
///
/// A title process carries none: its thread runs in problem state for
/// its whole life, and every privileged form refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorState {
    /// The machine state register.
    ///
    /// [PPC-Book3 p:10 s:2.2.3] The MSR is 64 bits and defines the state of the processor.
    pub msr: u64,
    /// Save/restore register 0: the address `rfid` returns to.
    ///
    /// [PPC-Book3 p:13 s:2.3] `rfid` takes the next instruction address from SRR0 and the new MSR from SRR1.
    pub srr0: u64,
    /// Save/restore register 1: the MSR `rfid` restores. See [`Self::srr0`].
    pub srr1: u64,
    /// The data address register.
    ///
    /// [PPC-Book3 p:15 s:3.3.1] The DAR is 64 bits.
    pub dar: u64,
    /// The data storage interrupt status register.
    ///
    /// [PPC-Book3 p:16 s:3.3.2] The DSISR is 32 bits.
    pub dsisr: u32,
    /// The decrementer's value at time base [`Self::dec_tb`].
    ///
    /// [PPC-Book3 p:80 s:6.3] The DEC is a 32-bit counter driven at the time-base frequency.
    pub dec: u32,
    /// The time base at which the decrementer held [`Self::dec`].
    pub dec_tb: u64,
    /// SPRG0 through SPRG3.
    ///
    /// [PPC-Book3 p:16 s:3.3.3] SPRG0-3 are 64-bit registers for privileged software, and accessing them has no side effect.
    pub sprg: [u64; 4],
    /// The segment lookaside buffer.
    pub slb: [SlbEntry; PPE_SLB_ENTRIES],
}

impl SupervisorState {
    /// The registers at the entry of supervisor code: `msr` as given,
    /// every other register 0, and every SLB entry invalid.
    pub fn new(msr: u64) -> Self {
        Self {
            msr,
            srr0: 0,
            srr1: 0,
            dar: 0,
            dsisr: 0,
            dec: 0,
            dec_tb: 0,
            sprg: [0; 4],
            slb: [SlbEntry::default(); PPE_SLB_ENTRIES],
        }
    }

    /// Whether MSR\[PR\] is 1.
    pub fn problem_state(&self) -> bool {
        self.msr & MSR_PR != 0
    }

    /// The decrementer at time base `tb`.
    ///
    /// [PPC-Book3 p:80 s:6.3] The DEC counts down at the time-base frequency.
    pub fn dec_at(&self, tb: u64) -> u32 {
        self.dec.wrapping_sub(tb.wrapping_sub(self.dec_tb) as u32)
    }

    /// The state-hash lane of these registers: [`supervisor_lane`] over
    /// [`Self::words`].
    pub fn digest(&self) -> u64 {
        supervisor_lane(&self.words())
    }

    /// Every register as one 64-bit word, in declaration order, each SLB
    /// entry as its effective half and then its virtual half.
    pub fn words(&self) -> [u64; SUPERVISOR_WORDS] {
        let mut out = [0u64; SUPERVISOR_WORDS];
        let head = [
            self.msr,
            self.srr0,
            self.srr1,
            self.dar,
            u64::from(self.dsisr),
            u64::from(self.dec),
            self.dec_tb,
            self.sprg[0],
            self.sprg[1],
            self.sprg[2],
            self.sprg[3],
        ];
        out[..head.len()].copy_from_slice(&head);
        for (k, e) in self.slb.iter().enumerate() {
            out[head.len() + 2 * k] = e.esid_v;
            out[head.len() + 2 * k + 1] = e.vsid_flags;
        }
        out
    }
}
