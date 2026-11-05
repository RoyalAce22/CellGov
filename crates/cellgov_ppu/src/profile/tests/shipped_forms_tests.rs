//! The retirements of each shipped form, and their rows in the report.

use super::{shipped_forms, OperandProfile, ProfileReport};
use crate::shadow::PredecodedShadow;

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// `lwz r3, 8(r1)`.
const LWZ: u32 = (32 << 26) | (3 << 21) | (1 << 16) | 8;
/// `cmpwi cr0, r3, 42`: fuses with the `lwz` before it.
const CMPWI: u32 = (11 << 26) | (3 << 16) | 42;
/// `li r5, 7`.
const LI: u32 = (14 << 26) | (5 << 21) | 7;
/// `addi r5, r5, 1`: no quickening applies.
const ADDI: u32 = (14 << 26) | (5 << 21) | (5 << 16) | 1;

/// The fused pair at 0 runs three times and the `li` at 8 twice. A branch
/// reaches the consumed slot at 4 once, and the `addi` at 12 runs once.
fn profile() -> OperandProfile {
    let mut profile = OperandProfile::default();
    for pc in [0, 4, 8, 12, 0, 4, 0, 4, 8, 4] {
        profile.record(pc);
    }
    profile
}

#[test]
fn shipped_forms_count_each_variant_by_its_retirements_most_retired_first() {
    let text = bytes(&[LWZ, CMPWI, LI, ADDI]);
    let shadow = PredecodedShadow::build(0, &text);

    let forms = shipped_forms(&profile(), &shadow, text.as_slice());

    assert_eq!(
        forms,
        vec![("LwzCmpwi".to_string(), 3), ("Li".to_string(), 2)],
        "the pair counts its head and ranks above the li; the consumed slot \
         and the addi are no form"
    );
}

#[test]
fn report_units_carries_the_shipped_forms_of_the_units_run() {
    use crate::PpuExecutionUnit;
    use cellgov_event::UnitId;
    use cellgov_exec::{ExecutionContext, ExecutionUnit};
    use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
    use cellgov_time::Budget;

    /// `sc`.
    const SC: u32 = 0x4400_0002;
    let code = [LWZ, CMPWI, LI, ADDI, SC];
    let text = bytes(&code);
    let mut mem = GuestMemory::new(0x1000);
    let range = ByteRange::new(GuestAddr::new(0), text.len() as u64).unwrap();
    mem.apply_commit(range, &text).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(PredecodedShadow::build(0, &text));
    unit.set_profile_mode(true);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert_eq!(
        unit.operand_profile().retired(),
        5,
        "precondition: ran to the sc"
    );

    let report = super::report_units([&unit], &mem);

    assert_eq!(
        report.shipped_forms,
        vec![("Li".to_string(), 1), ("LwzCmpwi".to_string(), 1)]
    );
}

#[test]
fn the_report_writes_a_row_per_shipped_form() {
    let text = bytes(&[LWZ, CMPWI, LI, ADDI]);
    let mut report = ProfileReport::new(&profile(), text.as_slice(), &text, None);
    report.shipped_forms = vec![("Li".to_string(), 3), ("LwzCmpwi".to_string(), 2)];

    let json = report.to_json();

    assert!(
        json.ends_with(
            "\"shipped\": null,\n  \"shipped_forms\": [\
             \n    {\"form\": \"Li\", \"dynamic\": 3},\
             \n    {\"form\": \"LwzCmpwi\", \"dynamic\": 2}\
             \n  ]\n}\n"
        ),
        "{json}"
    );
}
