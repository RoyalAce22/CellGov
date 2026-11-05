//! A fused pair whose second half reads memory after its first half
//! accessed a mapped window ends the batch after that first half. The two
//! instructions unfused do the same, and the second half then reads the
//! window after the runtime updates it. A fused pair whose second half
//! reads no memory retires both halves.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};

const RESERVED: u64 = 0x1_0000;
const WINDOW: u64 = 0x2_0000;
/// A writable doubleword in the code region, above the code.
const OUTSIDE: u64 = 0x800;

/// A DS-form doubleword access: `ld` (xo 0), `ldu` (xo 1) or `std`.
fn ds(op: u32, rt: u32, ra: u32, d: u16, xo: u32) -> u32 {
    (op << 26) | (rt << 21) | (ra << 16) | u32::from(d) | xo
}

/// A D-form access: `lwz`, `lbz` or `cmpwi`.
fn d_form(op: u32, rt: u32, ra: u32, d: u16) -> u32 {
    (op << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn ld(rt: u32, ra: u32, d: u16) -> u32 {
    ds(58, rt, ra, d, 0)
}
fn ldu(rt: u32, ra: u32, d: u16) -> u32 {
    ds(58, rt, ra, d, 1)
}
fn std_(rs: u32, ra: u32, d: u16) -> u32 {
    ds(62, rs, ra, d, 0)
}
fn lwz(rt: u32, ra: u32, d: u16) -> u32 {
    d_form(32, rt, ra, d)
}
fn lbz(rt: u32, ra: u32, d: u16) -> u32 {
    d_form(34, rt, ra, d)
}
/// `cmpwi cr0, rA, si`.
fn cmpwi(ra: u32, si: u16) -> u32 {
    d_form(11, 0, ra, si)
}

/// `addi r4, r4, 1`.
const ADDI_R4: u32 = (14 << 26) | (4 << 21) | (4 << 16) | 1;

/// A two-word head, the fused pair the shadow must build from it, and its
/// name for failure messages.
struct Head {
    name: &'static str,
    words: [u32; 2],
    fuses_as: fn(&FusedPair) -> bool,
}

/// Every pair whose second half reads memory after its first half
/// accessed memory. Each half addresses memory through r1 or r5, at `ea`
/// or at most 8 bytes above it.
fn reads_after_access_heads() -> [Head; 6] {
    [
        Head {
            name: "ld; ld (LdLd)",
            words: [ld(6, 5, 0), ld(7, 5, 8)],
            fuses_as: |p| matches!(p, FusedPair::LdLd { .. }),
        },
        Head {
            name: "ldu; ldu",
            words: [ldu(6, 5, 0), ldu(7, 5, 8)],
            fuses_as: |p| matches!(p, FusedPair::LduLdu { .. }),
        },
        Head {
            name: "lwz; lwz",
            words: [lwz(6, 5, 0), lwz(7, 5, 4)],
            fuses_as: |p| matches!(p, FusedPair::LwzLwz { .. }),
        },
        Head {
            name: "std; lwz",
            words: [std_(6, 1, 0), lwz(7, 5, 8)],
            fuses_as: |p| matches!(p, FusedPair::StdLwz { .. }),
        },
        Head {
            name: "ld; ld (LdLdAny)",
            words: [ld(6, 5, 8), ld(7, 5, 0)],
            fuses_as: |p| matches!(p, FusedPair::LdLdAny { .. }),
        },
        Head {
            name: "lbz; lbz",
            words: [lbz(6, 5, 0), lbz(7, 5, 1)],
            fuses_as: |p| matches!(p, FusedPair::LbzLbz { .. }),
        },
    ]
}

/// Pairs that access memory in their first half and read none in their
/// second.
fn no_read_after_access_heads() -> [Head; 2] {
    [
        Head {
            name: "lwz; cmpwi",
            words: [lwz(6, 5, 0), cmpwi(6, 5)],
            fuses_as: |p| matches!(p, FusedPair::LwzCmpwi { .. }),
        },
        Head {
            name: "std; std",
            words: [std_(6, 5, 0), std_(7, 5, 8)],
            fuses_as: |p| matches!(p, FusedPair::StdStd { .. }),
        },
    ]
}

/// Runs `head; addi r4, r4, 1` with r1 = r5 = `ea` and a window mapped,
/// in one batch of budget 4. Returns the instructions it retired.
fn retired(head: &Head, ea: u64) -> u64 {
    let code = [head.words[0], head.words[1], ADDI_R4];
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            RESERVED,
            0x4_0000,
            "reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap(),
        &bytes,
    )
    .unwrap();
    mem.map_window(WINDOW, 0x1_0000, "window").unwrap();
    let shadow = shadow::PredecodedShadow::build(0, &bytes);
    // An unfused head ends the batch after its first half with or without
    // the split rule, so each case needs the shadow to fuse the head.
    assert!(
        matches!(shadow.get(0), Some(PpuInstruction::Fused(p)) if (head.fuses_as)(&p)),
        "precondition: {} fuses as the expected pair, got {:?}",
        head.name,
        shadow.get(0)
    );
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_gpr(1, ea);
    unit.state_mut().set_gpr(5, ea);
    unit.run_until_yield(
        Budget::new(4),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    unit.retired_instructions()
}

#[test]
fn a_fused_pair_that_reads_after_a_window_access_ends_the_batch_after_its_first_half() {
    for head in &reads_after_access_heads() {
        assert_eq!(retired(head, WINDOW + 0x10), 1, "{}", head.name);
    }
}

#[test]
fn a_fused_pair_that_reads_after_an_access_outside_every_window_runs_the_batch_on() {
    for head in &reads_after_access_heads() {
        assert_eq!(retired(head, OUTSIDE), 3, "{}", head.name);
    }
}

#[test]
fn a_fused_pair_that_reads_nothing_after_a_window_access_retires_both_halves() {
    for head in &no_read_after_access_heads() {
        assert_eq!(retired(head, WINDOW + 0x10), 2, "{}", head.name);
    }
}
