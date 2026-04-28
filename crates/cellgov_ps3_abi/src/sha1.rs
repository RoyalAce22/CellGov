//! Const-fn SHA-1 used by the [`crate::nid_const`] macro to verify
//! that a hex NID literal agrees with the SHA-1-of-name-and-salt
//! computation at compile time.
//!
//! Only the bits of SHA-1 needed for the PS3 NID derivation are
//! implemented. Input names plus salt must fit in 247 bytes (the
//! 256-byte working buffer minus the 9-byte minimum SHA-1 padding);
//! every NID name in the workspace lookup table is well under that.

const H0: u32 = 0x67452301;
const H1: u32 = 0xEFCDAB89;
const H2: u32 = 0x98BADCFE;
const H3: u32 = 0x10325476;
const H4: u32 = 0xC3D2E1F0;

/// Suffix appended to a function name before SHA-1 for named-export
/// NID derivation. Hex value `0x6759659904250490566427499489741A`
/// matches the documented PS3 algorithm.
const NAMED_EXPORT_SALT: [u8; 16] = [
    0x67, 0x59, 0x65, 0x99, 0x04, 0x25, 0x04, 0x90, 0x56, 0x64, 0x27, 0x49, 0x94, 0x89, 0x74, 0x1A,
];

const fn sha1_compress(state: [u32; 5], block: &[u8; 64]) -> [u32; 5] {
    let mut w = [0u32; 80];
    let mut i = 0;
    while i < 16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
        i += 1;
    }
    while i < 80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        i += 1;
    }
    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut t = 0;
    while t < 80 {
        let (f, k) = if t < 20 {
            ((b & c) | (!b & d), 0x5A82_7999u32)
        } else if t < 40 {
            (b ^ c ^ d, 0x6ED9_EBA1u32)
        } else if t < 60 {
            ((b & c) | (b & d) | (c & d), 0x8F1B_BCDCu32)
        } else {
            (b ^ c ^ d, 0xCA62_C1D6u32)
        };
        let temp = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(k)
            .wrapping_add(w[t]);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = temp;
        t += 1;
    }
    [
        state[0].wrapping_add(a),
        state[1].wrapping_add(b),
        state[2].wrapping_add(c),
        state[3].wrapping_add(d),
        state[4].wrapping_add(e),
    ]
}

const fn sha1_first_word(prefix: &[u8], suffix: &[u8]) -> u32 {
    let total = prefix.len() + suffix.len();
    assert!(total <= 247, "sha1: input exceeds 247-byte working buffer");

    let mut buf = [0u8; 256];
    let mut i = 0;
    while i < prefix.len() {
        buf[i] = prefix[i];
        i += 1;
    }
    let mut j = 0;
    while j < suffix.len() {
        buf[prefix.len() + j] = suffix[j];
        j += 1;
    }
    buf[total] = 0x80;

    // Padded length: smallest multiple of 64 such that the last 8 bytes
    // hold the message-length-in-bits (so the (total+1)-th byte through
    // the (padded-9)-th byte are zero).
    let padded = if total % 64 < 56 {
        (total / 64) * 64 + 64
    } else {
        (total / 64) * 64 + 128
    };

    let bit_len = (total as u64).wrapping_mul(8);
    let len_bytes = bit_len.to_be_bytes();
    let mut k = 0;
    while k < 8 {
        buf[padded - 8 + k] = len_bytes[k];
        k += 1;
    }

    let mut state = [H0, H1, H2, H3, H4];
    let mut bi = 0;
    while bi < padded / 64 {
        let mut block = [0u8; 64];
        let mut p = 0;
        while p < 64 {
            block[p] = buf[bi * 64 + p];
            p += 1;
        }
        state = sha1_compress(state, &block);
        bi += 1;
    }

    state[0]
}

/// PS3 NID derivation: first 4 bytes of `SHA-1(name || salt)`,
/// interpreted as a little-endian `u32`.
///
/// The first SHA-1 word is rendered big-endian per the digest format,
/// then the same four bytes are read back as little-endian, which is
/// equivalent to byte-reversing the word.
pub const fn nid_sha1(name: &str) -> u32 {
    sha1_first_word(name.as_bytes(), &NAMED_EXPORT_SALT).swap_bytes()
}

#[cfg(test)]
mod tests {
    use super::nid_sha1;

    #[test]
    fn anchor_cell_spurs_initialize() {
        assert_eq!(nid_sha1("cellSpursInitialize"), 0xacfc_8dbc);
    }

    #[test]
    fn anchor_cell_spurs_finalize() {
        assert_eq!(nid_sha1("cellSpursFinalize"), 0xca4c_4600);
    }

    #[test]
    fn anchor_cell_spurs_add_workload() {
        assert_eq!(nid_sha1("cellSpursAddWorkload"), 0x6972_6aa2);
    }

    #[test]
    fn anchor_cell_gcm_init_body() {
        assert_eq!(nid_sha1("_cellGcmInitBody"), 0x15ba_e46b);
    }

    #[test]
    fn anchor_sys_lwmutex_create() {
        assert_eq!(nid_sha1("sys_lwmutex_create"), 0x2f85_c0ef);
    }

    const _COMPILE_TIME_ANCHOR: () = {
        assert!(nid_sha1("cellSpursInitialize") == 0xacfc_8dbc);
        assert!(nid_sha1("cellSpursFinalize") == 0xca4c_4600);
    };
}
