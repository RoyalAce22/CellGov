//! A report reads each retired word from the region that holds it, and
//! counts as unresolved each retirement whose word is missing or does not
//! decode.

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

use super::{report_units, InsnClass, OperandProfile, ProfileReport, WordSource};
use crate::shadow::PredecodedShadow;
use crate::PpuExecutionUnit;

/// The base of a code region that does not start at 0.
const CODE: u64 = 0x1_0000;
const SIZE: usize = 0x1_0000;

/// `addi r3, r3, 1`.
const ADDI: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `sc`.
const SC: u32 = 0x4400_0002;

/// A region at 0 and a code region at `CODE`, with `words` at `CODE`.
fn memory(words: &[u32]) -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, SIZE, "main", PageSize::Page64K),
        Region::new(CODE, SIZE, "code", PageSize::Page64K),
    ])
    .unwrap();
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let range = ByteRange::new(GuestAddr::new(CODE), bytes.len() as u64).unwrap();
    mem.apply_commit(range, &bytes).unwrap();
    mem
}

#[test]
fn a_run_in_a_region_other_than_the_one_at_zero_counts_every_retirement_in_a_class() {
    let mem = memory(&[ADDI, ADDI, SC]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = CODE;
    unit.set_profile_mode(true);
    unit.run_until_yield(
        Budget::new(8),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(
        unit.operand_profile().retired(),
        3,
        "precondition: ran to the sc"
    );

    let report = report_units([&unit], &mem);

    assert_eq!(report.unresolved, 0);
    let classed: u64 = report.insns.iter().map(|&(_, dynamic, _)| dynamic).sum();
    assert_eq!(classed, 3, "{:?}", report.insns);
}

#[test]
fn a_retirement_at_an_address_no_region_holds_counts_as_unresolved() {
    let mem = memory(&[ADDI]);
    let mut profile = OperandProfile::default();
    for pc in [CODE, CODE, 0x9_0000] {
        profile.record(pc);
    }

    let report = ProfileReport::new(&profile, &mem, &[], None);

    assert_eq!((report.retired, report.unresolved), (3, 1));
    let classed: u64 = report.insns.iter().map(|&(_, dynamic, _)| dynamic).sum();
    assert_eq!(classed, 2, "the held word counts; the unheld one does not");
    assert!(
        report
            .to_json()
            .starts_with("{\n  \"retired\": 3,\n  \"unresolved\": 1,\n  \"insns\": ["),
        "{}",
        report.to_json()
    );
}

#[test]
fn a_word_across_two_adjacent_regions_reads_from_both() {
    let mut mem = memory(&[]);
    let edge = CODE + SIZE as u64;
    mem.install_region(edge, SIZE, "next", PageSize::Page64K)
        .unwrap();
    let range = ByteRange::new(GuestAddr::new(edge - 2), 4).unwrap();
    mem.apply_commit(range, &ADDI.to_be_bytes()).unwrap();

    assert_eq!(mem.word(edge - 2), Some(ADDI));
    assert_eq!(mem.word(edge + SIZE as u64 - 2), None);
}

#[test]
fn a_shadow_from_zero_over_memory_fenced_below_the_text_still_counts_the_text() {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 2 * SIZE, "main", PageSize::Page64K)])
            .unwrap();
    let range = ByteRange::new(GuestAddr::new(CODE), 4).unwrap();
    mem.apply_commit(range, &ADDI.to_be_bytes()).unwrap();
    let shadow = PredecodedShadow::build(0, mem.as_bytes());
    mem.unmap_below(CODE);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);

    let report = report_units([&unit], &mem);

    let addi = InsnClass::of(ADDI).unwrap();
    let words = report
        .insns
        .iter()
        .find(|(class, _, _)| *class == addi)
        .map(|&(_, _, stat)| stat);
    assert_eq!(words, Some(1), "{:?}", report.insns);
}

#[test]
fn a_retirement_whose_word_does_not_decode_counts_as_unresolved() {
    let mem = memory(&[ADDI]);
    let mut profile = OperandProfile::default();
    for pc in [CODE, CODE + 4] {
        profile.record(pc);
    }

    let report = ProfileReport::new(&profile, &mem, &[], None);

    assert_eq!((report.retired, report.unresolved), (2, 1));
}

#[test]
fn merging_two_reports_sums_their_unresolved_retirements() {
    let mem = memory(&[ADDI]);
    let unheld = |pcs: &[u64]| {
        let mut profile = OperandProfile::default();
        for &pc in pcs {
            profile.record(pc);
        }
        ProfileReport::new(&profile, &mem, &[], None)
    };
    let mut merged = unheld(&[CODE, 0x9_0000]);
    let other = unheld(&[0x9_0000, 0x9_0004]);
    assert_eq!(
        (merged.unresolved, other.unresolved),
        (1, 2),
        "precondition: one and two unheld retirements"
    );

    merged.merge(other);

    assert_eq!((merged.retired, merged.unresolved), (4, 3));
}
