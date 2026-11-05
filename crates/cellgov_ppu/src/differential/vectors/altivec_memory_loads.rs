//! Spec-derived vectors for the AltiVec-memory load family (`lvsl`,
//! `lvsr`, `lvebx`, `lvehx`, `lvewx`, `lvxl`).
//!
//! For the element-indexed loads (`lvebx` / `lvehx` / `lvewx`) the
//! PEM leaves the other lanes undefined. The cases assert the full
//! 16-byte VRT the console gives: the whole aligned quadword.

use super::super::{InstructionCase, MemorySnapshot};
use super::{case_keep_memory, state_with_two_gprs};

/// Encode an X-form primary-31 instruction `(vt/vs, ra, rb, xo)`.
fn xform(vt_or_vs: u8, ra: u8, rb: u8, xo: u32) -> u32 {
    (31u32 << 26)
        | ((vt_or_vs as u32 & 0x1F) << 21)
        | ((ra as u32 & 0x1F) << 16)
        | ((rb as u32 & 0x1F) << 11)
        | (xo << 1)
}

const BASE_ADDR: u64 = 0x4000;

fn memory_with(bytes: Vec<u8>) -> MemorySnapshot {
    MemorySnapshot {
        base: BASE_ADDR,
        bytes,
    }
}

fn padded(offset: usize, payload: &[u8], total_len: usize) -> MemorySnapshot {
    let mut bytes = vec![0u8; total_len];
    bytes[offset..offset + payload.len()].copy_from_slice(payload);
    memory_with(bytes)
}

/// Spec-derived cases for the entire AltiVec-memory load family.
pub fn cases() -> Vec<InstructionCase> {
    let mut v = Vec::new();
    v.extend(lvsl_cases());
    v.extend(lvsr_cases());
    v.extend(lvebx_cases());
    v.extend(lvehx_cases());
    v.extend(lvewx_cases());
    v.extend(lvxl_cases());
    v
}

/// `lvsl`: permute control vector. Bytes 0..=15 of VRT = sh, sh+1, ..., sh+15
/// where `sh = EA[60:63]`. No memory read.
///
/// [AltiVec-PEM p:6-21 s:6.2] lvsl VRT, RA, RB: build shift-left permute control.
fn lvsl_cases() -> Vec<InstructionCase> {
    let raw = xform(/*vt*/ 1, /*ra*/ 4, /*rb*/ 5, 6);
    let mut cases = Vec::new();

    static LABELS: [&str; 16] = [
        "lvsl_sh_0",
        "lvsl_sh_1",
        "lvsl_sh_2",
        "lvsl_sh_3",
        "lvsl_sh_4",
        "lvsl_sh_5",
        "lvsl_sh_6",
        "lvsl_sh_7",
        "lvsl_sh_8",
        "lvsl_sh_9",
        "lvsl_sh_10",
        "lvsl_sh_11",
        "lvsl_sh_12",
        "lvsl_sh_13",
        "lvsl_sh_14",
        "lvsl_sh_15",
    ];
    for sh in 0u64..16 {
        let initial = state_with_two_gprs(4, BASE_ADDR, 5, sh);
        let mut expected = initial.clone();
        let mut bytes = [0u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = sh as u8 + i as u8;
        }
        expected.vr[1] = u128::from_be_bytes(bytes);
        cases.push(case_keep_memory(
            LABELS[sh as usize],
            raw,
            initial,
            MemorySnapshot::empty(),
            expected,
            MemorySnapshot::empty(),
            "[AltiVec-PEM p:6-21 s:6.2] VRT[i] = sh + i for i in 0..16",
        ));
    }

    cases
}

/// `lvsr`: shift-right companion. `VRT[i] = 16 + i - sh` for `i` in `0..16`.
///
/// [AltiVec-PEM p:6-22 s:6.2] lvsr VRT, RA, RB: build shift-right permute control.
fn lvsr_cases() -> Vec<InstructionCase> {
    let raw = xform(2, 4, 5, 38);
    let mut cases = Vec::new();

    static LABELS: [&str; 16] = [
        "lvsr_sh_0",
        "lvsr_sh_1",
        "lvsr_sh_2",
        "lvsr_sh_3",
        "lvsr_sh_4",
        "lvsr_sh_5",
        "lvsr_sh_6",
        "lvsr_sh_7",
        "lvsr_sh_8",
        "lvsr_sh_9",
        "lvsr_sh_10",
        "lvsr_sh_11",
        "lvsr_sh_12",
        "lvsr_sh_13",
        "lvsr_sh_14",
        "lvsr_sh_15",
    ];
    for sh in 0u64..16 {
        let initial = state_with_two_gprs(4, BASE_ADDR, 5, sh);
        let mut expected = initial.clone();
        let mut bytes = [0u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (16i32 + i as i32 - sh as i32) as u8;
        }
        expected.vr[2] = u128::from_be_bytes(bytes);
        cases.push(case_keep_memory(
            LABELS[sh as usize],
            raw,
            initial,
            MemorySnapshot::empty(),
            expected,
            MemorySnapshot::empty(),
            "[AltiVec-PEM p:6-22 s:6.2] VRT[i] = 16 + i - sh for i in 0..16",
        ));
    }

    cases
}

/// One element load at each offset in `labels`, into a line of distinct
/// bytes with a distinct prior VRT. Every case expects the whole line in
/// VRT. The `ppu_lve_lanes` console capture shows that the console fills
/// the lanes the load does not name from the aligned quadword.
fn element_load_cases(
    vt: u8,
    xo: u32,
    labels: &[(u64, &'static str)],
    citation: &'static str,
) -> Vec<InstructionCase> {
    let raw = xform(vt, 4, 5, xo);
    let line: [u8; 16] = std::array::from_fn(|i| 0x10 + i as u8);
    let mem = padded(0, &line, 16);
    labels
        .iter()
        .map(|&(ea_offset, label)| {
            let mut initial = state_with_two_gprs(4, BASE_ADDR, 5, ea_offset);
            initial.vr[vt as usize] = 0xE0E1_E2E3_E4E5_E6E7_E8E9_EAEB_ECED_EEEF;
            let mut expected = initial.clone();
            expected.vr[vt as usize] = u128::from_be_bytes(line);
            case_keep_memory(
                label,
                raw,
                initial,
                mem.clone(),
                expected,
                mem.clone(),
                citation,
            )
        })
        .collect()
}

/// `lvebx`: byte load at EA into byte position (EA & 0xF) of VRT.
///
/// [AltiVec-PEM p:6-14 s:6.2] lvebx VRT, RA, RB: 1-byte element load.
fn lvebx_cases() -> Vec<InstructionCase> {
    element_load_cases(
        3,
        7,
        &[
            (0, "lvebx_offset_0"),
            (1, "lvebx_offset_1"),
            (7, "lvebx_offset_7"),
            (15, "lvebx_offset_15"),
        ],
        "[AltiVec-PEM p:6-14 s:6.2] byte at EA -> VRT[8m..8m+7]; the console fills the other lanes from the aligned quadword",
    )
}

/// `lvehx`: halfword load at (EA & ~1) into halfword position
/// `(EA & 0xE)` of VRT.
///
/// [AltiVec-PEM p:6-16 s:6.2] lvehx VRT, RA, RB: 2-byte element load, EA aligned down to halfword.
fn lvehx_cases() -> Vec<InstructionCase> {
    element_load_cases(
        4,
        39,
        &[
            (0, "lvehx_offset_0"),
            (2, "lvehx_offset_2"),
            (8, "lvehx_offset_8"),
            (14, "lvehx_offset_14"),
        ],
        "[AltiVec-PEM p:6-16 s:6.2] halfword at (EA&~1) -> VRT lane (EA&0xE); the console fills the other lanes from the aligned quadword",
    )
}

/// `lvewx`: word load at (EA & ~3) into word position `(EA & 0xC)` of VRT.
///
/// [AltiVec-PEM p:6-17 s:6.2] lvewx VRT, RA, RB: 4-byte element load, EA aligned down to word.
fn lvewx_cases() -> Vec<InstructionCase> {
    element_load_cases(
        5,
        71,
        &[
            (0, "lvewx_offset_0"),
            (4, "lvewx_offset_4"),
            (8, "lvewx_offset_8"),
            (12, "lvewx_offset_12"),
        ],
        "[AltiVec-PEM p:6-17 s:6.2] word at (EA&~3) -> VRT lane (EA&0xC); the console fills the other lanes from the aligned quadword",
    )
}

/// `lvxl`: identical to `lvx` -- the "Last" suffix is a cache LRU hint.
///
/// [AltiVec-PEM p:6-23 s:6.2] lvxl VRT, RA, RB: same semantics as lvx with LRU hint.
fn lvxl_cases() -> Vec<InstructionCase> {
    let raw = xform(6, 4, 5, 359);
    let payload = [
        0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ];
    let mem = padded(0, &payload, 32);
    let initial = state_with_two_gprs(4, BASE_ADDR, 5, 0);
    let mut expected = initial.clone();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&payload);
    expected.vr[6] = u128::from_be_bytes(bytes);
    vec![case_keep_memory(
        "lvxl_aligned_at_offset_0",
        raw,
        initial,
        mem.clone(),
        expected,
        mem,
        "[AltiVec-PEM p:6-23 s:6.2] lvxl: 16-byte aligned load; LRU hint ignored",
    )]
}

#[cfg(test)]
#[path = "tests/altivec_memory_loads_tests.rs"]
mod tests;
