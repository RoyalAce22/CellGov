use super::*;

#[test]
fn hex_accepts_both_prefix_spellings_and_none() {
    for spelling in ["0x10", "0X10", "10"] {
        assert_eq!(hex_u64(spelling).unwrap(), 0x10, "{spelling}");
    }
    assert_eq!(hex_u64("deadbeef").unwrap(), 0xdead_beef);
}

#[test]
fn hex_rejects_a_prefix_with_no_digits() {
    assert!(hex_u64("0x").is_err());
    assert!(hex_u64("").is_err());
    assert!(hex_u64("0xZZ").is_err());
}

#[test]
fn a_u32_flag_refuses_a_value_that_does_not_fit() {
    assert_eq!(hex_u32("0xffffffff").unwrap(), u32::MAX);
    assert!(hex_u32("0x100000000").is_err());
}

#[test]
fn a_step_takes_decimal_unless_it_carries_a_hex_prefix() {
    assert_eq!(step_count("16").unwrap(), 16);
    assert_eq!(step_count("0x16").unwrap(), 0x16);
    assert!(step_count("dead").is_err());
}

#[test]
fn a_csv_entry_parser_refuses_an_empty_entry() {
    assert_eq!(hex_addr("0x1").unwrap(), 1);
    let refusals = [
        hex_addr("").err(),
        dump_mem_fault_range("").err(),
        patch_byte_pair("").err(),
    ];
    for refusal in &refusals {
        assert!(
            matches!(refusal, Some(CliArgError::EmptyCsvEntry)),
            "{refusal:?}"
        );
    }
}

#[test]
fn a_fault_range_defaults_its_length_and_bounds_it() {
    assert_eq!(dump_mem_fault_range("0x100").unwrap(), (0x100, 0x40));
    assert_eq!(dump_mem_fault_range("0x100:8").unwrap(), (0x100, 8));
    for malformed in [
        "0x100:0",
        "0x100:20000",
        "0x100:8:8",
        "0xffffffffffffffff:8",
    ] {
        assert!(dump_mem_fault_range(malformed).is_err(), "{malformed}");
    }
}

#[test]
fn a_patch_byte_pair_wants_one_equals_and_two_hex_fields() {
    assert_eq!(patch_byte_pair("0x100=ff").unwrap(), (0x100, 0xff));
    for malformed in ["0x100", "=ff", "0x100=", "0x100=f=f", "0x100=fff"] {
        assert!(patch_byte_pair(malformed).is_err(), "{malformed}");
    }
}

#[test]
fn a_checkpoint_takes_the_two_keywords_and_a_pc_literal() {
    use crate::game::manifest::CheckpointTrigger;
    assert_eq!(
        checkpoint("process-exit").unwrap(),
        CheckpointTrigger::ProcessExit
    );
    assert_eq!(
        checkpoint("first-rsx-write").unwrap(),
        CheckpointTrigger::FirstRsxWrite
    );
    assert_eq!(
        checkpoint("pc=0x1000").unwrap(),
        CheckpointTrigger::Pc(0x1000)
    );
    assert!(checkpoint("halt").is_err());
}
