use std::path::Path;

use super::super::{KeyVault, SelfClass};

fn h(byte: u8, len: usize) -> String {
    format!("{byte:02x}").repeat(len)
}

#[test]
fn constructor_catalog_preserves_components_and_registers_lv2_range() {
    let text = format!(
        "void KeyVault::LoadSelfLv2Keys() {{\n  sk_LV2_arr.emplace_back(0x0003006000000000, 0x0003006100000000, 0x0, KEY, \"{}\", \"{}\", \"{}\", \"{}\", 0x01);\n}}\n",
        h(0x11, 32), h(0x22, 16), h(0x33, 40), h(0x44, 21),
    );
    let vault = KeyVault::parse(Path::new("key_vault.cpp"), text.as_bytes()).unwrap();
    let material: Vec<_> = vault.material().collect();
    assert_eq!(material.len(), 1);
    assert_eq!(material[0].kind, "lv2");
    assert_eq!(material[0].label, "0003006000000000-0003006100000000");
    assert_eq!(material[0].components.len(), 4);
    assert_eq!(vault.labels(SelfClass::Lv2), ["3.60-3.61"]);
    assert!(vault.ignored().is_empty(), "{:?}", vault.ignored());
}
