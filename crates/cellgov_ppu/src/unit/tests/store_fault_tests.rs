//! A store to a byte no region holds, or that a region refuses writes
//! to, faults at the store.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{DataAccessFault, ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_time::Budget;

use crate::instruction::FusedPair;

/// The end of the one writable region, at 0.
const FLAT_END: u64 = 0x1000;
/// An address between the two regions.
const UNMAPPED: u64 = 0x8000;
/// The base of a region that refuses writes.
const RSX: u64 = 0x1_0000;

/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;

fn memory(code: &[u32]) -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, FLAT_END as usize, "flat", PageSize::Page4K),
        Region::with_access(
            RSX,
            0x1000,
            "rsx",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    for (i, word) in code.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    mem
}

/// Runs `insn` at 0 with `addi r3` behind it, r5 = `ea`, and the
/// other address registers zero.
fn run_at(insn: u32, ea: u64) -> (cellgov_exec::ExecutionStepResult, u64) {
    let mem = memory(&[insn, ADDI_R3]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, ea);
    let ctx = ExecutionContext::new(&mem);
    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut Vec::new());
    (r, unit.state().gpr[3])
}

fn store_record(refused_by: Option<&'static str>) -> Option<DataAccessFault> {
    Some(DataAccessFault {
        store: true,
        refused_by,
    })
}

/// `op rS, 0(r5)` for a D-form store.
fn d_form(op: u32) -> u32 {
    (op << 26) | (6 << 21) | (5 << 16)
}

/// `xo rS, r5, r0` for an X-form store under primary opcode 31.
fn x_form(xo: u32) -> u32 {
    (31 << 26) | (6 << 21) | (5 << 16) | (xo << 1)
}

#[test]
fn every_store_form_to_an_unmapped_address_faults_at_the_store() {
    let forms = [
        ("stb", d_form(38)),
        ("sth", d_form(44)),
        ("stw", d_form(36)),
        ("std", d_form(62)),
        ("stfd", d_form(54)),
        ("stmw", (47 << 26) | (30 << 21) | (5 << 16)),
        (
            "stswi",
            (31 << 26) | (6 << 21) | (5 << 16) | (8 << 11) | (725 << 1),
        ),
        ("stvx", x_form(231)),
        ("dcbz", (31 << 26) | (5 << 16) | (1014 << 1)),
    ];
    for (name, insn) in forms {
        let (r, r3) = run_at(insn, UNMAPPED);
        assert_eq!(r.yield_reason, YieldReason::Fault, "{name}");
        assert_eq!(r.local_diagnostics.pc, Some(0), "{name}");
        assert_eq!(r.local_diagnostics.faulting_ea, Some(UNMAPPED), "{name}");
        assert_eq!(
            r.local_diagnostics.data_access,
            store_record(None),
            "{name}"
        );
        assert_eq!(r3, 0, "{name}: the addi behind the store never ran");
    }
}

#[test]
fn a_fused_store_pair_to_an_unmapped_address_faults_at_its_first_store() {
    // std r6,0(r5); std r7,8(r5) fuse into one pair.
    let first = d_form(62);
    let second = (62 << 26) | (7 << 21) | (5 << 16) | 8;
    let bytes: Vec<u8> = [first, second]
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect();
    let shadow = shadow::PredecodedShadow::build(0, &bytes);
    assert!(
        matches!(
            shadow.get(0),
            Some(instruction::PpuInstruction::Fused(FusedPair::StdStd { .. }))
        ),
        "precondition: the two stores fuse"
    );
    let mem = memory(&[first, second, ADDI_R3]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_gpr(5, UNMAPPED);
    let ctx = ExecutionContext::new(&mem);

    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut Vec::new());

    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(r.local_diagnostics.pc, Some(0));
    assert_eq!(r.local_diagnostics.faulting_ea, Some(UNMAPPED));
    assert_eq!(r.local_diagnostics.data_access, store_record(None));
}

#[test]
fn a_store_that_runs_past_the_end_of_a_region_names_the_first_byte_past_it() {
    // std at the region's last word: four bytes in, four past the end.
    let (r, _) = run_at(d_form(62), FLAT_END - 4);

    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(r.local_diagnostics.faulting_ea, Some(FLAT_END));
    assert_eq!(r.local_diagnostics.data_access, store_record(None));
}

#[test]
fn a_store_into_a_region_that_refuses_writes_faults_at_the_store() {
    let (r, r3) = run_at(d_form(36), RSX + 0x40);

    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(r.local_diagnostics.pc, Some(0));
    assert_eq!(r.local_diagnostics.faulting_ea, Some(RSX + 0x40));
    assert_eq!(r.local_diagnostics.data_access, store_record(Some("rsx")));
    assert_eq!(r3, 0);
}

#[test]
fn a_load_from_the_same_addresses_records_a_load() {
    let lwz = (32 << 26) | (6 << 21) | (5 << 16);
    let ld = (58 << 26) | (6 << 21) | (5 << 16);
    let load = Some(DataAccessFault {
        store: false,
        refused_by: None,
    });

    let (r, _) = run_at(lwz, UNMAPPED);
    assert_eq!(r.local_diagnostics.faulting_ea, Some(UNMAPPED));
    assert_eq!(r.local_diagnostics.data_access, load);

    let (r, _) = run_at(ld, FLAT_END - 4);
    assert_eq!(r.local_diagnostics.faulting_ea, Some(FLAT_END));
    assert_eq!(r.local_diagnostics.data_access, load);

    let (r, r3) = run_at(lwz, RSX + 0x40);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "the region reads as zero"
    );
    assert_eq!(r3, 1);
}
