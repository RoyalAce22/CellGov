//! A minimal PARAM.SFO emitter for synthetic title trees.
//!
//! The layout follows [`cellgov_ps3_abi::format::param_sfo`], so the production
//! parser reads what this writes.

use cellgov_ps3_abi::format::param_sfo::{
    SFO_FMT_STRING, SFO_FORMAT_VERSION, SFO_HEADER_LEN, SFO_INDEX_LEN, SFO_MAGIC,
};

/// Builds a PARAM.SFO that holds `entries` as string values, in the
/// order given.
///
/// # Panics
///
/// When the key table passes 64 KiB: `key_off` is a 16-bit field, so a
/// longer table has no encoding.
#[must_use]
pub fn build_param_sfo(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut key_table = Vec::new();
    let mut key_offs = Vec::new();
    for (k, _) in entries {
        let key_off =
            u16::try_from(key_table.len()).expect("PARAM.SFO key table exceeds the u16 key_off");
        key_offs.push(key_off);
        key_table.extend_from_slice(k.as_bytes());
        key_table.push(0);
    }
    while key_table.len() % 4 != 0 {
        key_table.push(0);
    }

    let mut data_table = Vec::new();
    let mut recs: Vec<(u16, u32, u32, u32)> = Vec::new(); // key_off, len, max, data_off
    for (i, (_, v)) in entries.iter().enumerate() {
        let data_off = data_table.len() as u32;
        let mut b = v.as_bytes().to_vec();
        b.push(0);
        let l = b.len() as u32;
        data_table.extend_from_slice(&b);
        recs.push((key_offs[i], l, l, data_off));
    }

    let n = entries.len();
    let off_key_table = (SFO_HEADER_LEN + n * SFO_INDEX_LEN) as u32;
    let off_data_table = off_key_table + key_table.len() as u32;

    let mut buf = Vec::new();
    buf.extend_from_slice(&SFO_MAGIC);
    buf.extend_from_slice(&SFO_FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&off_key_table.to_le_bytes());
    buf.extend_from_slice(&off_data_table.to_le_bytes());
    buf.extend_from_slice(&(n as u32).to_le_bytes());
    for (key_off, len, max, data_off) in &recs {
        buf.extend_from_slice(&key_off.to_le_bytes());
        buf.extend_from_slice(&SFO_FMT_STRING.to_le_bytes());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&max.to_le_bytes());
        buf.extend_from_slice(&data_off.to_le_bytes());
    }
    buf.extend_from_slice(&key_table);
    buf.extend_from_slice(&data_table);
    buf
}
