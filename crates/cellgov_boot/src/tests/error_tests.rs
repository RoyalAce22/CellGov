//! The `u32` narrowing every boot-computed guest address passes
//! through.

use super::{narrow_u32, NarrowError};

#[test]
fn the_widest_value_a_u32_field_holds_is_accepted() {
    assert_eq!(narrow_u32("code_floor", 0), Ok(0));
    assert_eq!(narrow_u32("code_floor", u64::from(u32::MAX)), Ok(u32::MAX));
}

#[test]
fn one_past_the_widest_value_is_refused_rather_than_truncated() {
    let value = u64::from(u32::MAX) + 1;
    assert_eq!(
        narrow_u32("alloc_base", value),
        Err(NarrowError {
            label: "alloc_base",
            value,
        }),
        "0x1_0000_0000 truncates to 0, which aliases the null page",
    );
}

#[test]
fn the_refusal_names_the_value_that_did_not_fit() {
    let msg = narrow_u32("alloc_base", 0x1_0000_0000)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("alloc_base"), "got: {msg}");
    assert!(msg.contains("0x0000000100000000"), "got: {msg}");
}
