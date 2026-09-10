use super::*;

#[test]
fn lookup_returns_empty_module_for_libstdcxx_symbols() {
    let (m, n) = lookup(0x003395d9).expect("_Feraise is in nid_db");
    assert_eq!(m, "");
    assert_eq!(n, "_Feraise");
}

#[test]
fn lookup_misses_a_nid_the_table_lacks() {
    assert_eq!(lookup(0xffff_ffff), None);
}
