//! Each row's form agrees with its opcode width, the prefix index agrees
//! with a scan of the map, and a mnemonic finds its own row.

use super::*;

/// [SPU-ISA p:28 s:2.3] RR and RI7 carry an 11-bit opcode and RRR a 4-bit one.
/// [SPU-ISA p:29 s:2.3] RI10, RI16 and RI18 carry 8-, 9- and 7-bit opcodes.
#[test]
fn every_form_matches_its_opcode_width() {
    for row in SPU_OPCODE_MAP {
        let widths: &[u8] = match row.form {
            SpuForm::Rr | SpuForm::Ri7 => &[11],
            SpuForm::Rrr => &[4],
            SpuForm::Ri8 => &[10],
            SpuForm::Ri10 => &[8],
            SpuForm::Ri16 => &[9],
            SpuForm::Ri18 | SpuForm::Hint => &[7],
        };
        assert!(widths.contains(&row.width), "{}", row.mnemonic);
    }
    let ri7 = SPU_OPCODE_MAP
        .iter()
        .filter(|row| row.form == SpuForm::Ri7)
        .count();
    // [SPU-ISA p:230 s:9] dftsv carries its I7 test mask in the RI7 position.
    assert_eq!(ri7, 19);
}

#[test]
fn the_prefix_index_agrees_with_a_scan_of_the_map() {
    for prefix in 0..2048u32 {
        for low in [0, 0x001F_FFFF] {
            let raw = prefix << 21 | low;
            let scanned = SPU_OPCODE_MAP.iter().position(|row| row.matches(raw));
            assert_eq!(row_for(raw).map(|(index, _)| index), scanned, "{raw:#010x}");
        }
    }
}

#[test]
fn a_mnemonic_names_its_own_row_and_nothing_else() {
    for (index, row) in SPU_OPCODE_MAP.iter().enumerate() {
        assert_eq!(row_named(row.mnemonic), Some(index));
    }
    assert_eq!(row_named("fsqrt"), None);
    assert_eq!(row_named(""), None);
}
