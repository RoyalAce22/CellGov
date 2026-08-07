//! AES keys for PS3 firmware and SELF decryption.
//!
//! PUP / SCEPKG scalar keys live in [`cellgov_ps3_abi::sce`] and are
//! re-exported here for backward compatibility. APP keys mirror
//! RPCS3's `KeyVault::LoadSelfAPPKeys` table. Revisions 0x0012 and
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

/// APP-key table, sorted by revision. Gaps at 0x0012 and 0x0015.
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

/// NPDRM SELF keys, selected by revision tag for NPDRM-wrapped
/// binaries. The APP table does not apply: each revision carries
/// distinct ERK/RIV for the two paths.
const NPDRM_KEYS: &[(u16, SelfKey)] = &[
    (
        0x0001,
        key(
            "F9EDD0301F770FABBA8863D9897F0FEA6551B09431F61312654E28F43533EA6B",
            "A551CCB4A42C37A734A2B4F9657D5540",
        ),
    ),
    (
        0x0002,
        key(
            "8E737230C80E66AD0162EDDD32F1F774EE5E4E187449F19079437A508FCF9C86",
            "7AAECC60AD12AED90C348D8C11D2BED5",
        ),
    ),
    (
        0x0003,
        key(
            "1B715B0C3E8DC4C1A5772EBA9C5D34F7CCFE5B82025D453F3167566497239664",
            "E31E206FBB8AEA27FAB0D9A2FFB6B62F",
        ),
    ),
    (
        0x0004,
        key(
            "BB4DBF66B744A33934172D9F8379A7A5EA74CB0F559BB95D0E7AECE91702B706",
            "ADF7B207A15AC601110E61DDFC210AF6",
        ),
    ),
    (
        0x0006,
        key(
            "8B4C52849765D2B5FA3D5628AFB17644D52B9FFEE235B4C0DB72A62867EAA020",
            "05719DF1B1D0306C03910ADDCE4AF887",
        ),
    ),
    (
        0x0007,
        key(
            "3946DFAA141718C7BE339A0D6C26301C76B568AEBC5CD52652F2E2E0297437C3",
            "E4897BE553AE025CDCBF2B15D1C9234E",
        ),
    ),
    (
        0x0009,
        key(
            "0786F4B0CA5937F515BDCE188F569B2EF3109A4DA0780A7AA07BD89C3350810A",
            "04AD3C2F122A3B35E804850CAD142C6D",
        ),
    ),
    (
        0x000A,
        key(
            "03C21AD78FBB6A3D425E9AAB1298F9FD70E29FD4E6E3A3C151205DA50C413DE4",
            "0A99D4D4F8301A88052D714AD2FB565E",
        ),
    ),
    (
        0x000C,
        key(
            "357EBBEA265FAEC271182D571C6CD2F62CFA04D325588F213DB6B2E0ED166D92",
            "D26E6DD2B74CD78E866E742E5571B84F",
        ),
    ),
    (
        0x000D,
        key(
            "337A51416105B56E40D7CAF1B954CDAF4E7645F28379904F35F27E81CA7B6957",
            "8405C88E042280DBD794EC7E22B74002",
        ),
    ),
    (
        0x000F,
        key(
            "135C098CBE6A3E037EBE9F2BB9B30218DDE8D68217346F9AD33203352FBB3291",
            "4070C898C2EAAD1634A288AA547A35A8",
        ),
    ),
    (
        0x0010,
        key(
            "4B3CD10F6A6AA7D99F9B3A660C35ADE08EF01C2C336B9E46D1BB5678B4261A61",
            "C0F2AB86E6E0457552DB50D7219371C5",
        ),
    ),
    (
        0x0013,
        key(
            "265C93CF48562EC5D18773BEB7689B8AD10C5EB6D21421455DEBC4FB128CBF46",
            "8DEA5FF959682A9B98B688CEA1EF4A1D",
        ),
    ),
    (
        0x0016,
        key(
            "7910340483E419E55F0D33E4EA5410EEEC3AF47814667ECA2AA9D75602B14D4B",
            "4AD981431B98DFD39B6388EDAD742A8E",
        ),
    ),
    (
        0x0019,
        key(
            "FBDA75963FE690CFF35B7AA7B408CF631744EDEF5F7931A04D58FD6A921FFDB3",
            "F72C1D80FFDA2E3BF085F4133E6D2805",
        ),
    ),
    (
        0x001C,
        key(
            "8103EA9DB790578219C4CEDF0592B43064A7D98B601B6C7BC45108C4047AA80F",
            "246F4B8328BE6A2D394EDE20479247C5",
        ),
    ),
];

/// Look up the NPDRM SELF key for a revision tag. Returns `None` for
/// revisions that have no NPDRM entry (notably 0x0000, 0x0005, 0x0008,
/// 0x000B, 0x000E, 0x0011, 0x0012, 0x0014, 0x0015, 0x0017, 0x0018,
/// 0x001A, 0x001B, and any revision past 0x001C).
pub fn npdrm_key_for_revision(revision: u16) -> Option<SelfKey> {
    NPDRM_KEYS
        .iter()
        .find(|(rev, _)| *rev == revision)
        .map(|(_, k)| *k)
}

#[cfg(test)]
#[path = "tests/crypto_tests.rs"]
mod tests;
