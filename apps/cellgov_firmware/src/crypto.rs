//! AES keys for PS3 firmware and SELF decryption.
//!
//! PUP / SCEPKG scalar keys live in [`cellgov_ps3_abi::sce`] and are
//! re-exported here for backward compatibility. APP keys mirror
//! `KeyVault::LoadSelfAPPKeys` in
//! `tools/rpcs3-src/rpcs3/Crypto/key_vault.cpp`. Revisions 0x0012 and
//! 0x0015 have no entry in either source.

pub use cellgov_ps3_abi::sce::{PUP_KEY, SCEPKG_ERK, SCEPKG_RIV};

/// AES-256-CBC key + IV pair for one SELF revision's APP-key slot.
#[derive(Copy, Clone)]
pub struct SelfKey {
    /// 32-byte AES-256 key.
    pub erk: [u8; 0x20],
    /// 16-byte initialization vector.
    pub riv: [u8; 0x10],
}

const fn nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => panic!("non-hex character in compile-time hex constant"),
    }
}

const fn hex32(s: &str) -> [u8; 0x20] {
    assert!(
        s.len() == 64,
        "ERK hex literal must be exactly 64 characters"
    );
    let bytes = s.as_bytes();
    let mut out = [0u8; 0x20];
    let mut i = 0;
    while i < 0x20 {
        out[i] = (nibble(bytes[i * 2]) << 4) | nibble(bytes[i * 2 + 1]);
        i += 1;
    }
    out
}

const fn hex16(s: &str) -> [u8; 0x10] {
    assert!(
        s.len() == 32,
        "RIV hex literal must be exactly 32 characters"
    );
    let bytes = s.as_bytes();
    let mut out = [0u8; 0x10];
    let mut i = 0;
    while i < 0x10 {
        out[i] = (nibble(bytes[i * 2]) << 4) | nibble(bytes[i * 2 + 1]);
        i += 1;
    }
    out
}

const fn key(erk_hex: &str, riv_hex: &str) -> SelfKey {
    SelfKey {
        erk: hex32(erk_hex),
        riv: hex16(riv_hex),
    }
}

/// Sorted by revision. Gaps at 0x0012 and 0x0015.
const APP_KEYS: &[(u16, SelfKey)] = &[
    (
        0x0000,
        key(
            "95F50019E7A68E341FA72EFDF4D60ED376E25CF46BB48DFDD1F080259DC93F04",
            "4A0955D946DB70D691A640BB7FAECC4C",
        ),
    ),
    (
        0x0001,
        key(
            "79481839C406A632BDB4AC093D73D99AE1587F24CE7E69192C1CD0010274A8AB",
            "6F0F25E1C8C4B7AE70DF968B04521DDA",
        ),
    ),
    (
        0x0002,
        key(
            "4F89BE98DDD43CAD343F5BA6B1A133B0A971566F770484AAC20B5DD1DC9FA06A",
            "90C127A9B43BA9D8E89FE6529E25206F",
        ),
    ),
    (
        0x0003,
        key(
            "C1E6A351FCED6A0636BFCB6801A0942DB7C28BDFC5E0A053A3F52F52FCE9754E",
            "E0908163F457576440466ACAA443AE7C",
        ),
    ),
    (
        0x0004,
        key(
            "838F5860CF97CDAD75B399CA44F4C214CDF951AC795298D71DF3C3B7E93AAEDA",
            "7FDBB2E924D182BB0D69844ADC4ECA5B",
        ),
    ),
    (
        0x0005,
        key(
            "C109AB56593DE5BE8BA190578E7D8109346E86A11088B42C727E2B793FD64BDC",
            "15D3F191295C94B09B71EBDE088A187A",
        ),
    ),
    (
        0x0006,
        key(
            "6DFD7AFB470D2B2C955AB22264B1FF3C67F180983B26C01615DE9F2ECCBE7F41",
            "24BD1C19D2A8286B8ACE39E4A37801C2",
        ),
    ),
    (
        0x0007,
        key(
            "945B99C0E69CAF0558C588B95FF41B232660ECB017741F3218C12F9DFDEEDE55",
            "1D5EFBE7C5D34AD60F9FBC46A5977FCE",
        ),
    ),
    (
        0x0008,
        key(
            "2C9E8969EC44DFB6A8771DC7F7FDFBCCAF329EC3EC070900CABB23742A9A6E13",
            "5A4CEFD5A9C3C093D0B9352376D19405",
        ),
    ),
    (
        0x0009,
        key(
            "F69E4A2934F114D89F386CE766388366CDD210F1D8913E3B973257F1201D632B",
            "F4D535069301EE888CC2A852DB654461",
        ),
    ),
    (
        0x000A,
        key(
            "29805302E7C92F204009161CA93F776A072141A8C46A108E571C46D473A176A3",
            "5D1FAB844107676ABCDFC25EAEBCB633",
        ),
    ),
    (
        0x000B,
        key(
            "A4C97402CC8A71BC7748661FE9CE7DF44DCE95D0D58938A59F47B9E9DBA7BFC3",
            "E4792F2B9DB30CB8D1596077A13FB3B5",
        ),
    ),
    (
        0x000C,
        key(
            "9814EFFF67B7074D1B263BF85BDC8576CE9DEC914123971B169472A1BC2387FA",
            "D43B1FA8BE15714B3078C23908BB2BCA",
        ),
    ),
    (
        0x000D,
        key(
            "03B4C421E0C0DE708C0F0B71C24E3EE04306AE7383D8C5621394CCB99FF7A194",
            "5ADB9EAFE897B54CB1060D6885BE22CF",
        ),
    ),
    (
        0x000E,
        key(
            "39A870173C226EB8A3EEE9CA6FB675E82039B2D0CCB22653BFCE4DB013BAEA03",
            "90266C98CBAA06C1BF145FF760EA1B45",
        ),
    ),
    (
        0x000F,
        key(
            "FD52DFA7C6EEF5679628D12E267AA863B9365E6DB95470949CFD235B3FCA0F3B",
            "64F50296CF8CF49CD7C643572887DA0B",
        ),
    ),
    (
        0x0010,
        key(
            "A5E51AD8F32FFBDE808972ACEE46397F2D3FE6BC823C8218EF875EE3A9B0584F",
            "7A203D5112F799979DF0E1B8B5B52AA4",
        ),
    ),
    (
        0x0011,
        key(
            "0F8EAB8884A51D092D7250597388E3B8B75444AC138B9D36E5C7C5B8C3DF18FD",
            "97AF39C383E7EF1C98FA447C597EA8FE",
        ),
    ),
    (
        0x0013,
        key(
            "DBF62D76FC81C8AC92372A9D631DDC9219F152C59C4B20BFF8F96B64AB065E94",
            "CB5DD4BE8CF115FFB25801BC6086E729",
        ),
    ),
    (
        0x0014,
        key(
            "491B0D72BB21ED115950379F4564CE784A4BFAABB00E8CB71294B192B7B9F88E",
            "F98843588FED8B0E62D7DDCB6F0CECF4",
        ),
    ),
    (
        0x0016,
        key(
            "A106692224F1E91E1C4EBAD4A25FBFF66B4B13E88D878E8CD072F23CD1C5BF7C",
            "62773C70BD749269C0AFD1F12E73909E",
        ),
    ),
    (
        0x0017,
        key(
            "4E104DCE09BA878C75DA98D0B1636F0E5F058328D81419E2A3D22AB0256FDF46",
            "954A86C4629E116532304A740862EF85",
        ),
    ),
    (
        0x0018,
        key(
            "1F876AB252DDBCB70E74DC4A20CD8ED51E330E62490E652F862877E8D8D0F997",
            "BF8D6B1887FA88E6D85C2EDB2FBEC147",
        ),
    ),
    (
        0x0019,
        key(
            "3236B9937174DF1DC12EC2DD8A318A0EA4D3ECDEA5DFB4AC1B8278447000C297",
            "6153DEE781B8ADDC6A439498B816DC46",
        ),
    ),
    (
        0x001A,
        key(
            "5EFD1E9961462794E3B9EF2A4D0C1F46F642AAE053B5025504130590E66F19C9",
            "1AC8FA3B3C90F8FDE639515F91B58327",
        ),
    ),
    (
        0x001B,
        key(
            "66637570D1DEC098467DB207BAEA786861964D0964D4DBAF89E76F46955D181B",
            "9F7B5713A5ED59F6B35CD8F8A165D4B8",
        ),
    ),
    (
        0x001C,
        key(
            "CFF025375BA0079226BE01F4A31F346D79F62CFB643CA910E16CF60BD9092752",
            "FD40664E2EBBA01BF359B0DCDF543DA4",
        ),
    ),
    (
        0x001D,
        key(
            "D202174EB65A62048F3674B59EF6FE72E1872962F3E1CD658DE8D7AF71DA1F3E",
            "ACB9945914EBB7B9A31ECE320AE09F2D",
        ),
    ),
];

/// Look up the APP key for a SELF container's revision tag. Returns
/// `None` for the unknown-revision gaps (0x0012, 0x0015) and for any
/// revision past the highest known entry.
pub fn app_key_for_revision(revision: u16) -> Option<SelfKey> {
    APP_KEYS
        .iter()
        .find(|(rev, _)| *rev == revision)
        .map(|(_, k)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_key_lookup_returns_expected_entries() {
        let k = app_key_for_revision(0x0000).expect("revision 0x0000 present");
        assert_eq!(k.erk[0], 0x95);
        assert_eq!(k.riv[0], 0x4A);
    }

    #[test]
    fn app_key_lookup_handles_gaps() {
        assert!(app_key_for_revision(0x0012).is_none());
        assert!(app_key_for_revision(0x0015).is_none());
    }

    #[test]
    fn app_key_lookup_returns_none_past_table() {
        assert!(app_key_for_revision(0x9999).is_none());
    }

    #[test]
    fn revision_001c_key_matches_rpcs3() {
        let k = app_key_for_revision(0x001C).expect("revision 0x001C present");
        assert_eq!(k.erk[0], 0xCF);
        assert_eq!(k.erk[1], 0xF0);
        assert_eq!(k.riv[0], 0xFD);
    }
}
