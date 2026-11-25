//! Effective to real address translation: real addressing mode, or the
//! SLB and the hashed page table.

use cellgov_ps3_abi::hw::ppu_mmu::{
    HASH_BITS, HASH_INDEX_MIN_BITS, PPE_PTE_AVPN, PPE_PTE_LP, PPE_PTE_RPN, PPE_REAL_ADDRESS_BITS,
    PTEG_BYTES, PTE_BYTES, PTE_C, PTE_G, PTE_H, PTE_I, PTE_L, PTE_N, PTE_PP, PTE_V,
    REAL_MODE_IGNORED_EA, SDR1_HTABORG, SDR1_HTABSIZE, SEGMENT_LOG2, SLB_KP, SLB_KS, SLB_L, SLB_N,
    SMALL_PAGE_LOG2,
};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, PPE_SLB_LP};

use crate::exec::PpuInterrupt;
use crate::supervisor::{SlbEntry, SupervisorState};

/// What an access does with the storage it reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// A load, or any access that reads without writing.
    Load,
    /// A store, `dcbz` or conditional store.
    Store,
    /// An instruction fetch.
    Fetch,
}

/// One block of effective addresses that maps onto contiguous real
/// addresses with one set of attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Translation {
    /// The block's first effective address.
    pub ea: u64,
    /// The real address of [`Self::ea`].
    pub ra: u64,
    /// The block's length in bytes.
    pub len: u64,
    /// Whether a store may write the block.
    pub writable: bool,
    /// Whether the block is caching-inhibited storage.
    pub inhibited: bool,
    /// Whether the block's page table entry has its change bit set.
    ///
    /// A real mode block has no entry and holds `true`.
    pub changed: bool,
}

impl Translation {
    /// The real address of `ea`, or `None` when the block does not hold it.
    pub fn real(&self, ea: u64) -> Option<u64> {
        let off = ea.wrapping_sub(self.ea);
        (off < self.len).then(|| self.ra + off)
    }
}

/// What the PP bits and the key allow.
///
/// [PPC-Book3 p:45 s:4.10.1] Figure 30: with key 0, PP 11 is read only and the rest read/write; with key 1, PP 00 is no access, 10 read/write, and 01 and 11 read only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authority {
    None,
    ReadOnly,
    ReadWrite,
}

fn authority(key: bool, pp: u64) -> Authority {
    match (key, pp) {
        (false, 0b11) | (true, 0b01) | (true, 0b11) => Authority::ReadOnly,
        (true, 0b00) => Authority::None,
        _ => Authority::ReadWrite,
    }
}

/// The translation of `ea` for `access` on a thread with supervisor
/// registers `sup`.
///
/// `pteg` reads the 128-byte page table entry group at a real address,
/// or gives `None` where no storage is. The search finds no matching
/// entry in a group that `pteg` gives as `None`.
///
/// The translation does not set a PTE's R or C bit.
///
/// # Errors
///
/// The storage or segment interrupt the access takes:
/// - a real mode access at or past the real mode limit: the storage
///   interrupt with the protection cause;
/// - no SLB entry for the segment: the segment interrupt;
/// - no page table entry for the page: the storage interrupt with the
///   not-found cause;
/// - a page the access may not reach: the storage interrupt with the
///   protection or no-execute cause.
pub fn translate(
    ea: u64,
    access: Access,
    sup: &SupervisorState,
    pteg: impl Fn(u64) -> Option<[u8; PTEG_BYTES]>,
) -> Result<Translation, PpuInterrupt> {
    let relocate = match access {
        Access::Fetch => MSR_IR,
        Access::Load | Access::Store => MSR_DR,
    };
    if sup.msr & relocate == 0 {
        real_mode(ea, access, sup)
    } else {
        virtual_mode(ea, access, sup, pteg)
    }
}

/// The interrupt for a refused access at `ea`.
fn refusal(ea: u64, access: Access, cause: Refusal) -> PpuInterrupt {
    match access {
        Access::Fetch => match cause {
            Refusal::Segment => PpuInterrupt::InstructionSegment,
            Refusal::NotFound => PpuInterrupt::InstructionStorage {
                no_execute: false,
                protection: false,
            },
            Refusal::NoExecute => PpuInterrupt::InstructionStorage {
                no_execute: true,
                protection: false,
            },
            Refusal::Protection => PpuInterrupt::InstructionStorage {
                no_execute: false,
                protection: true,
            },
        },
        Access::Load | Access::Store => {
            let store = access == Access::Store;
            match cause {
                Refusal::Segment => PpuInterrupt::DataSegment { ea, store },
                Refusal::NotFound | Refusal::NoExecute => PpuInterrupt::DataStorage {
                    ea,
                    store,
                    protection: false,
                },
                Refusal::Protection => PpuInterrupt::DataStorage {
                    ea,
                    store,
                    protection: true,
                },
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Refusal {
    Segment,
    NotFound,
    NoExecute,
    Protection,
}

/// Offset real mode: EA bits 2:63 below the limit, ORed with the offset,
/// and cut to the PPE's 42 real address bits.
///
/// The block is the aligned span that holds `ea`, as long as the limit or
/// the real address space, whichever is smaller. The partition's offset
/// is a multiple of the limit, so the OR is an add.
///
/// [PPC-Book3 p:30 s:4.2.6.2] An EA whose bits 2:63 are at or above the RMLR limit causes a storage exception, and an EA below it reaches the low-order m bits of EA bits 2:63 ORed with the RMOR.
/// [PPC-Book3 p:46 s:4.10.2] Figure 31: with LPES1 = 1 and MSR\[HV\] = 0, an access below the limit is read/write and any other access is not permitted.
/// [CBE-Handbook p:98 s:4.2.8] Real mode ignores the EA bits above the real address range when it forms the RA.
/// [CBE-Handbook p:99 s:4.2.8.1] Table 4-9: a real mode access has PP 10 and C = 1.
fn real_mode(ea: u64, access: Access, sup: &SupervisorState) -> Result<Translation, PpuInterrupt> {
    let part = &sup.partition;
    if ea & !REAL_MODE_IGNORED_EA >= part.rm_limit() {
        return Err(refusal(ea, access, Refusal::Protection));
    }
    let ra_mask = (1u64 << PPE_REAL_ADDRESS_BITS) - 1;
    let len = part.rm_limit().min(ra_mask + 1);
    Ok(Translation {
        ea: ea & !(len - 1),
        ra: part.rm_offset() & ra_mask,
        len,
        writable: true,
        inhibited: false,
        changed: true,
    })
}

/// The SLB entry that translates `ea`.
///
/// [PPC-Book3 p:34 s:4.4.2] An SLB entry matches when V = 1 and its ESID equals EA bits 0:35.
fn segment(sup: &SupervisorState, ea: u64) -> Option<&SlbEntry> {
    let esid = ea >> SEGMENT_LOG2;
    sup.slb
        .iter()
        .find(|e| e.valid() && e.esid_v >> SEGMENT_LOG2 == esid)
}

fn virtual_mode(
    ea: u64,
    access: Access,
    sup: &SupervisorState,
    pteg: impl Fn(u64) -> Option<[u8; PTEG_BYTES]>,
) -> Result<Translation, PpuInterrupt> {
    let slbe = segment(sup, ea).ok_or_else(|| refusal(ea, access, Refusal::Segment))?;
    let large = slbe.vsid_flags & SLB_L != 0;
    let lp = slbe.vsid_flags & PPE_SLB_LP != 0;
    let p = if large {
        sup.partition.large_page_log2(lp)
    } else {
        SMALL_PAGE_LOG2
    };
    let d1 = search(sup.partition.sdr1(), slbe, ea, p, &pteg)
        .ok_or_else(|| refusal(ea, access, Refusal::NotFound))?;

    // [PPC-Book3 p:38 s:4.5.3] The N value is the PTE's N ORed with the SLB entry's N.
    // [PPC-Book3 p:45 s:4.10.1] A fetch is not permitted when N is 1 or the PTE's G is 1, and every access takes its Key from Kp in problem state and from Ks otherwise.
    if access == Access::Fetch && (slbe.vsid_flags & SLB_N != 0 || d1 & (PTE_N | PTE_G) != 0) {
        return Err(refusal(ea, access, Refusal::NoExecute));
    }
    let key_bit = if sup.problem_state() { SLB_KP } else { SLB_KS };
    let allowed = authority(slbe.vsid_flags & key_bit != 0, d1 & PTE_PP);
    let refused = match access {
        Access::Load | Access::Fetch => allowed == Authority::None,
        Access::Store => allowed != Authority::ReadWrite,
    };
    if refused {
        return Err(refusal(ea, access, Refusal::Protection));
    }

    // [PPC-Book3 p:38 s:4.5.3] RA is RPN bits 0:61-p followed by EA bits 64-p:63.
    // [CBE-Handbook p:79 s:4.2.1] Table 4-1: the PPE does not implement RA bits 2:21 and treats them as 0.
    let page_mask = (1u64 << p) - 1;
    Ok(Translation {
        ea: ea & !page_mask,
        ra: d1 & PPE_PTE_RPN & !page_mask,
        len: 1 << p,
        writable: allowed == Authority::ReadWrite,
        inhibited: d1 & PTE_I != 0,
        changed: d1 & PTE_C != 0,
    })
}

/// Doubleword 1 of the first PTE that translates `ea` in `slbe`, with pages of 2^`p` bytes.
///
/// [PPC-Book3 p:38 s:4.5.3] The primary hash XORs VPN bits 13:51 with the low-order 28-p bits of the VPN; the secondary hash is its one's complement. A PTEG's real address is HTABORG, the hash's bits 0:27 masked by HTABSIZE ORed into HTABORG's low-order bits, the hash's bits 28:38, and seven 0 bits.
/// [PPC-Book3 p:38 s:4.5.3] A PTE matches when H names its group, V = 1, AVPN bits 0:51+q equal VA bits 0:51+q with q = min(5, 28-p), and L equals the SLB entry's L.
/// [CBE-Handbook p:88 s:4.2.6.5] The PPE also requires PTE\[LP\] = SLBE\[LP\] when PTE\[L\] = 1; when several entries match and differ outside SW, H, AC, R and C, the translation is undefined.
///
/// When several entries match, the search takes the first in the PPE's
/// order: the primary group before the secondary, and in each group the
/// even slots before the odd. Entries that match may differ in R and C,
/// so the order decides the change bit the translation gives.
/// [CBE-Handbook p:87 s:4.2.6.4] The PPE searches PTEs 0, 2, 4 and 6 of a group before PTEs 1, 3, 5 and 7.
/// [PPC-Book3 p:38 s:4.5.3] Entries that match may differ in SW, H, AC, R and C, and one of them gives the translation and takes the R and C update.
fn search(
    sdr1: u64,
    slbe: &SlbEntry,
    ea: u64,
    p: u32,
    pteg: &impl Fn(u64) -> Option<[u8; PTEG_BYTES]>,
) -> Option<u64> {
    let vsid = slbe.vsid_flags >> 12;
    let offset = ea & ((1u64 << SEGMENT_LOG2) - 1);
    let hash_mask = (1u64 << HASH_BITS) - 1;
    let primary = (vsid ^ (offset >> p)) & hash_mask;
    let size_mask = (1u64 << (sdr1 & SDR1_HTABSIZE)) - 1;
    let index_low = (1u64 << HASH_INDEX_MIN_BITS) - 1;
    let pteg_shift = PTEG_BYTES.trailing_zeros();

    // VA bits 0:56, the AVPN's span, as doubleword 0 holds them.
    let va = (u128::from(vsid) << SEGMENT_LOG2) | u128::from(offset);
    let want = ((va >> 23) as u64) << 7;
    let q = 5.min(28 - p);
    let compare = PPE_PTE_AVPN & (!0u64 << (12 - q));
    let large = slbe.vsid_flags & SLB_L != 0;
    let lp = slbe.vsid_flags & PPE_SLB_LP != 0;

    for (secondary, hash) in [(false, primary), (true, !primary & hash_mask)] {
        let addr = (sdr1 & SDR1_HTABORG)
            | (((hash >> HASH_INDEX_MIN_BITS) & size_mask) << (HASH_INDEX_MIN_BITS + pteg_shift))
            | ((hash & index_low) << pteg_shift);
        let Some(group) = pteg(addr) else {
            continue;
        };
        let even = group.chunks_exact(PTE_BYTES).step_by(2);
        let odd = group.chunks_exact(PTE_BYTES).skip(1).step_by(2);
        for pte in even.chain(odd) {
            let d0 = u64::from_be_bytes(pte[..8].try_into().unwrap_or_default());
            let d1 = u64::from_be_bytes(pte[8..].try_into().unwrap_or_default());
            let hit = d0 & PTE_V != 0
                && (d0 & PTE_H != 0) == secondary
                && (d0 & PTE_L != 0) == large
                && (!large || (d1 & PPE_PTE_LP != 0) == lp)
                && (d0 ^ want) & compare == 0;
            if hit {
                return Some(d1);
            }
        }
    }
    None
}
