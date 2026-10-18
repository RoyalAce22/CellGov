use super::*;
use crate::hw::spu_isa::SPU_OPCODE_MAP;

#[test]
fn every_row_the_cbe_provides_has_a_table_entry_and_no_other_row_does() {
    let mut provided = 0;
    for (row, op) in SPU_OPCODE_MAP.iter().enumerate() {
        assert_eq!(
            SPU_ISSUE_TABLE[row].is_some(),
            op.on_cbe,
            "{}: a Table B-1 entry exactly when the CBE provides it",
            op.mnemonic
        );
        provided += usize::from(op.on_cbe);
    }
    assert_eq!(
        provided,
        SPU_OPCODE_MAP.len() - 5,
        "the five optional compares"
    );
}

fn issue(mnemonic: &str) -> SpuIssue {
    SPU_ISSUE_TABLE[row_named(mnemonic).expect("a row")].expect("an entry")
}

#[test]
fn table_b1_gives_each_class_its_pipeline_and_latency() {
    assert_eq!(issue("ai"), even(2));
    assert_eq!(issue("shl"), even(4));
    assert_eq!(issue("fma"), even(6));
    assert_eq!(issue("mpy"), even(7));
    assert_eq!(issue("dfa"), DOUBLE);
    assert_eq!(issue("shufb"), odd(4));
    assert_eq!(issue("lqd"), odd(6));
    assert_eq!(issue("rchcnt"), odd(6));
    assert_eq!(issue("brnz").pipe, SpuPipe::Odd);
    assert_eq!(issue("nop"), even(0));
    assert_eq!(issue("lnop"), odd(0));
}

#[test]
fn a_getllar_takes_the_measured_mean_in_spu_cycles() {
    // 172 time-base ticks over 32 runs, at 40 cycles a tick.
    assert_eq!(SPU_GETLLAR_CYCLES, 215);
}

#[test]
fn a_branch_miss_costs_one_cycle_more_into_the_second_half_of_a_fetch_line() {
    assert_eq!(branch_miss_cycles(0x1700), 18);
    assert_eq!(branch_miss_cycles(0x173C), 18);
    assert_eq!(branch_miss_cycles(0x1740), 19);
    assert_eq!(branch_miss_cycles(0x177C), 19);
    assert_eq!(branch_miss_cycles(0x1780), 18);
}
