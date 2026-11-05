use super::*;

const HEADER: &str = "id\tasm\twords\tinputs\trecord\tmay_end_process";

fn row(inputs: &str, record: &str) -> String {
    format!("a\tnop\t0x60000000\t{inputs}\t{record}\t0")
}

#[test]
fn a_row_refuses_a_wrong_field_count() {
    assert_eq!(parse_row("a\tnop\t0x60000000").unwrap_err(), "3 fields");
}

#[test]
fn a_row_refuses_no_words() {
    assert_eq!(parse_row("a\tnop\t-\t-\t-\t0").unwrap_err(), "no words");
}

#[test]
fn a_row_refuses_a_may_end_process_other_than_0_or_1() {
    assert_eq!(
        parse_row("a\tnop\t0x60000000\t-\t-\t2").unwrap_err(),
        "may_end_process \"2\""
    );
}

#[test]
fn a_row_refuses_an_input_without_a_value() {
    assert_eq!(parse_row(&row("r3", "-")).unwrap_err(), "input \"r3\"");
}

#[test]
fn an_input_refuses_a_value_wider_than_its_register() {
    assert_eq!(
        parse_row(&row("cr=0x1_0000_0000", "-")).unwrap_err(),
        "cr 0x100000000"
    );
    assert_eq!(
        parse_row(&row("fpscr=0x1_0000_0000", "-")).unwrap_err(),
        "fpscr 0x100000000"
    );
    assert_eq!(
        parse_row(&row("r3=0x1_0000_0000_0000_0000", "-")).unwrap_err(),
        "r3 0x10000000000000000"
    );
    let v = parse_row(&row("v3=0x1_0000_0000_0000_0000", "-")).expect("a VR takes 128 bits");
    assert_eq!(v.inputs, vec![(Reg::Vr(3), 1u128 << 64)]);
}

#[test]
fn an_input_loads_cr_xer_and_fpscr() {
    let case = parse_row(&row("cr=0x1,xer=0xffff_ffff_ffff_ffff,fpscr=0x3", "-")).expect("row");
    assert_eq!((case.cr, case.xer, case.fpscr), (1, u64::MAX, 3));
    assert!(case.inputs.is_empty());
}

#[test]
fn an_input_refuses_a_memory_doubleword_by_name() {
    assert_eq!(
        parse_row(&row("m0=0x1", "-")).unwrap_err(),
        "m0: memory is loaded with mem="
    );
}

#[test]
fn a_row_refuses_a_register_loaded_or_recorded_twice() {
    assert_eq!(
        parse_row(&row("r3=0x1,r3=0x2", "-")).unwrap_err(),
        "Gpr(3) loaded twice"
    );
    assert_eq!(
        parse_row(&row("-", "r3,r3")).unwrap_err(),
        "Gpr(3) recorded twice"
    );
}

#[test]
fn a_table_refuses_a_duplicate_id_on_the_later_row() {
    let text = format!("{HEADER}\n{}\n{}\n", row("-", "-"), row("-", "-"));
    assert_eq!(parse_rows(&text).unwrap_err(), "row 2: duplicate id \"a\"");
}

#[test]
fn a_table_refuses_a_wrong_or_missing_header() {
    assert_eq!(parse_rows("").unwrap_err(), "empty table");
    assert_eq!(parse_rows("id\tasm").unwrap_err(), "header \"id\\tasm\"");
}

#[test]
fn a_row_error_names_its_row() {
    let text = format!("{HEADER}\n{}\n{}\n", row("-", "-"), "b\tnop\t-\t-\t-\t0");
    assert_eq!(
        parse_table(&text).unwrap_err().to_string(),
        "row 2: no words"
    );
}
