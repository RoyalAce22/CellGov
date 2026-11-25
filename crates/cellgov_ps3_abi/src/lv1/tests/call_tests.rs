use super::*;

#[test]
fn the_table_ascends_with_no_repeated_number() {
    assert_eq!(LV1_CALLS.len(), 137);
    for pair in LV1_CALLS.windows(2) {
        assert!(pair[0].number < pair[1].number, "{pair:?}");
    }
}

#[test]
fn every_row_fits_the_hypercall_registers() {
    assert_eq!(LV1_CALLS.len(), 137);
    for call in LV1_CALLS {
        assert!(call.inputs <= MAX_INPUTS, "{call:?}");
        assert!(call.outputs <= MAX_OUTPUTS, "{call:?}");
    }
}

#[test]
fn lookup_finds_a_listed_number_and_misses_an_unlisted_one() {
    assert_eq!(lookup(232).map(|c| c.name), Some("get_rtc"));
    // Neither header lists 5.
    assert_eq!(lookup(5), None);
    assert_eq!(lookup(u64::MAX), None);
    // A number whose low 16 bits name a listed call is still unlisted.
    assert_eq!(lookup(0x1_0000 + 232), None);
}

/// The rows are the `LV1_CALL` rows of Linux `lv1call.h` (74, 91) and
/// of its OtherOS++ copy (105, 107, 109).
#[test]
fn the_floor_boot_calls_carry_their_header_numbers_names_and_counts() {
    assert_eq!(GET_LOGICAL_PARTITION_ID, 74);
    assert_eq!(READ_REPOSITORY_NODE, 91);
    assert_eq!(LOGICAL_CONSOLE_CONSTRUCT, 105);
    assert_eq!(LOGICAL_CONSOLE_SEND, 107);
    assert_eq!(LOGICAL_CONSOLE_FLUSH, 109);
    let row = |n: u64| lookup(n).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row(74),
        Some(("get_logical_partition_id", 0, 1, CallSource::Linux))
    );
    assert_eq!(
        row(91),
        Some(("read_repository_node", 5, 2, CallSource::Linux))
    );
    assert_eq!(
        row(105),
        Some((
            "undocumented_function_105",
            7,
            0,
            CallSource::OtherOsPlusPlus
        ))
    );
    assert_eq!(
        row(107),
        Some((
            "undocumented_function_107",
            6,
            1,
            CallSource::OtherOsPlusPlus
        ))
    );
    assert_eq!(
        row(109),
        Some((
            "undocumented_function_109",
            1,
            0,
            CallSource::OtherOsPlusPlus
        ))
    );
}
