//! AES keys and per-revision APP keys used to decrypt PS3 firmware and SELF files.

pub const PUP_KEY: [u8; 0x40] = [
    0xF4, 0x91, 0xAD, 0x94, 0xC6, 0x81, 0x10, 0x96, 0x91, 0x5F, 0xD5, 0xD2, 0x44, 0x81, 0xAE, 0xDC,
    0xED, 0xED, 0xBE, 0x6B, 0xE5, 0x13, 0x72, 0x4D, 0xD8, 0xF7, 0xB6, 0x91, 0xE8, 0x8A, 0x38, 0xF4,
    0xB5, 0x16, 0x2B, 0xFB, 0xEC, 0xBE, 0x3A, 0x62, 0x18, 0x5D, 0xD7, 0xC9, 0x4D, 0xA2, 0x22, 0x5A,
    0xDA, 0x3F, 0xBF, 0xCE, 0x55, 0x5B, 0x9E, 0xA9, 0x64, 0x98, 0x29, 0xEB, 0x30, 0xCE, 0x83, 0x66,
];

pub const SCEPKG_ERK: [u8; 0x20] = [
    0xA9, 0x78, 0x18, 0xBD, 0x19, 0x3A, 0x67, 0xA1, 0x6F, 0xE8, 0x3A, 0x85, 0x5E, 0x1B, 0xE9, 0xFB,
    0x56, 0x40, 0x93, 0x8D, 0x4D, 0xBC, 0xB2, 0xCB, 0x52, 0xC5, 0xA2, 0xF8, 0xB0, 0x2B, 0x10, 0x31,
];

pub const SCEPKG_RIV: [u8; 0x10] = [
    0x4A, 0xCE, 0xF0, 0x12, 0x24, 0xFB, 0xEE, 0xDF, 0x82, 0x45, 0xF8, 0xFF, 0x10, 0x21, 0x1E, 0x6E,
];

pub struct SelfKey {
    pub erk: [u8; 0x20],
    pub riv: [u8; 0x10],
}

fn hex_to_bytes_32(s: &str) -> [u8; 0x20] {
    let mut out = [0u8; 0x20];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
    }
    out
}

fn hex_to_bytes_16(s: &str) -> [u8; 0x10] {
    let mut out = [0u8; 0x10];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
    }
    out
}

pub fn app_key_for_revision(revision: u16) -> Option<SelfKey> {
    let (erk_hex, riv_hex) = match revision {
        0x0000 => (
            "95F50019E7A68E341FA72EFDF4D60ED376E25CF46BB48DFDD1F080259DC93F04",
            "4A0955D946DB70D691A640BB7FAECC4C",
        ),
        0x0001 => (
            "79481839C406A632BDB4AC093D73D99AE1587F24CE7E69192C1CD0010274A8AB",
            "6F0F25E1C8C4B7AE70DF968B04521DDA",
        ),
        0x0002 => (
            "4F89BE98DDD43CAD343F5BA6B1A133B0A971566F770484AAC20B5DD1DC9FA06A",
            "90C127A9B43BA9D8E89FE6529E25206F",
        ),
        0x0003 => (
            "C1E6A351FCED6A0636BFCB6801A0942DB7C28BDFC5E0A053A3F52F52FCE9754E",
            "E0908163F457576440466ACAA443AE7C",
        ),
        0x0004 => (
            "AA85431B301B4736DBABC3AC326AE57AE8A0124E1113A4E61BF9D99B1E8B0395",
            "C4A52999C0D735A47DC5B9DA5D3DDFC0",
        ),
        0x0005 => (
            "A66F97A50C49A3DE4DF3B7BE4C07240A8B67278F89B02EAD56E7E8B3D49B839B",
            "F44C27D3E3F4D8E10A9B67CDD8E986C0",
        ),
        0x0006 => (
            "08ABFD7E8ACC78C42E07CA64F2AEA5CC9D22E80D4281AFC1DEFD3BF3DACC717F",
            "D2C80B1680987B6D9B8FFF5A03CC70BA",
        ),
        0x0007 => (
            "17C4C421BF12D9A7A0F844E5D8F97C13BB313DC26C1D79ABAA8DBBA6CDBB30E5",
            "3C23F0E0A063E12F17C81B6B05DBFE3C",
        ),
        0x0008 => (
            "E89D06B59BDFD6ADB50BD9CD88F93E48405C0160ABD3C67B70E3C7DA2A33BA82",
            "C9AF373AA2578F2E4BE04DB3E9AA79D5",
        ),
        0x0009 => (
            "CA0A7DD5AD0948DFB7C22EFBAEF6F96E31CEFCD73F24ED10E70DDDBA949E7F07",
            "2F8B12C2CDCDF85BCED0ACA4F83B7BE0",
        ),
        0x000A => (
            "6AF2448D25E09EA90F5F3FF82DFFC1E5F04E7F7A40EE0DB1A3DEAE31FC52F13A",
            "DE23A55E9A5B41AC63A67C21BAC5F310",
        ),
        0x000B => (
            "F49D7E3CCDDCFEF0CC6F1AE4B5F2C1CA0B39B09B81F3F2A6CDE8DA4EF3F05A2",
            "FF76D92F79DE0DB2B2F42E1D6DECFEF4",
        ),
        0x000C => (
            "7339BF56A5D5B8DFBEB03B2C28DFAC07E77D45BCA3BC3C38B8B9DA6B73C1D139",
            "C4E1E0A2C9EA8DB02CD86B05AC821D71",
        ),
        0x000D => (
            "C8399B40FDBE43A00CDE424D66483A9BCABB09C9F95A4E73BFD63F5F963C8CC4",
            "47B32E3A8D1C83E2FF19FB3C57FA1A18",
        ),
        _ => return None,
    };
    Some(SelfKey {
        erk: hex_to_bytes_32(erk_hex),
        riv: hex_to_bytes_16(riv_hex),
    })
}
