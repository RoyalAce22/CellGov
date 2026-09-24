//! The secondary and indirect OPD table scanners.

use cellgov_mem::be::read_u32;

use super::phdr::pt_load_segments;
use super::process_param::pt_load_file_to_guest;

/// Secondary OPD pointer table located by 8-byte header signature.
///
/// The tables sit in the title's `.data`, outside the PRX_PARAM
/// `lib_stub_start..lib_stub_end` import area, so no header locates
/// them. The CRT0 walker patches each slot with an HLE OPD address at
/// runtime; the cross-runner classifier covers them under the same
/// `HleOpdSlot` rule as the primary table.
///
/// Observed layout in two retail title executables: header
/// `04 02 NN 00  00 NN 00 00` with NN a sequence-number byte, then
/// 0x60 bytes of slots; two adjacent tables per title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecondaryOpdTable {
    /// Guest virtual address of the table's first byte (header).
    pub guest_addr: u64,
    /// Total table size in bytes (header + slot array).
    pub size: u64,
}

/// Total size of one secondary OPD table (header + slot array).
pub const SECONDARY_OPD_TABLE_SIZE: u64 = 0x68;

/// Locate every secondary OPD table in `data` by header-signature
/// scan over the EBOOT file. Candidates outside any PT_LOAD file
/// range are rejected via the same filter [`find_sys_process_param`](super::find_sys_process_param)
/// uses, so stray byte sequences in section-header strings or
/// embedded assets cannot masquerade as real tables. Returns tables
/// in file-order; caller is responsible for merging adjacent extents
/// into a single classifier range if desired.
///
/// The scan stride is 4 bytes, the PPC OPD-pointer natural alignment.
pub fn find_secondary_opd_tables(data: &[u8]) -> Vec<SecondaryOpdTable> {
    let mut out = Vec::new();
    if data.len() < 8 {
        return out;
    }
    let mut i = 0usize;
    while i + 8 <= data.len() {
        let w0 = read_u32(data, i);
        let w1 = read_u32(data, i + 4);
        let w0_seq = (w0 >> 8) & 0xFF;
        let w1_seq = (w1 >> 16) & 0xFF;
        let header_match = (w0 & 0xFFFF_00FF) == 0x0402_0000
            && w0_seq != 0
            && (w1 & 0xFF00_FFFF) == 0
            && w1_seq != 0
            && w0_seq == w1_seq;
        if header_match {
            if let Some(guest_addr) = pt_load_file_to_guest(data, i) {
                out.push(SecondaryOpdTable {
                    guest_addr,
                    size: SECONDARY_OPD_TABLE_SIZE,
                });
                i += SECONDARY_OPD_TABLE_SIZE as usize;
                continue;
            }
        }
        i += 4;
    }
    out
}

/// One contiguous (id, ptr, opd_slot) triple-table observed in EBOOT
/// data. Field layout per row (4 bytes each):
///
/// - Bytes 0..4: caller-side identifier (counter, opcode, etc.).
/// - Bytes 4..8: pointer into the title's executable text segment.
/// - Bytes 8..12: OPD pointer slot. The CRT0 / PRX-link walker
///   rewrites this with an HLE OPD address at runtime; per-runner
///   addresses differ but the resolved function is equivalent.
///
/// Found by [`find_indirect_opd_tables`]: a run of consecutive
/// 12-byte rows where column 1 sits inside the executable PT_LOAD
/// range. WipEout's table at `data@0xc1110` is the driving
/// observation; SSHD-shaped titles may have a sibling table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndirectOpdTable {
    /// Guest virtual address of the table's first row's first byte.
    pub guest_addr: u64,
    /// Total table size in bytes (= `count * INDIRECT_OPD_TABLE_STRIDE`).
    pub size: u64,
}

/// Per-row stride of an indirect OPD table.
pub const INDIRECT_OPD_TABLE_STRIDE: u64 = 12;

/// Byte offset of the OPD pointer slot within one row.
pub const INDIRECT_OPD_TABLE_SLOT_OFFSET: u64 = 8;

/// Minimum consecutive rows required to claim an indirect-OPD-table
/// extent. Four rows is enough to suppress two-pointer coincidences
/// (a 16-byte block of identical structure) while still catching small
/// tables. WipEout's table is ~60 rows; far above threshold.
const INDIRECT_OPD_TABLE_MIN_ROWS: usize = 4;

/// Locate every indirect-OPD table in `data` by scanning for runs of
/// 12-byte rows whose column-1 (bytes 4..8) is a pointer into the
/// title's executable PT_LOAD range. Each detected run that meets the
/// internal row-count threshold is emitted as a single
/// [`IndirectOpdTable`]; callers derive the per-row OPD slot positions
/// at offset [`INDIRECT_OPD_TABLE_SLOT_OFFSET`].
///
/// The scan is 4-byte aligned and rejects matches outside any PT_LOAD
/// file range so stray byte sequences cannot masquerade as real tables.
pub fn find_indirect_opd_tables(data: &[u8]) -> Vec<IndirectOpdTable> {
    let mut out = Vec::new();
    let Ok(segs) = pt_load_segments(data) else {
        return out;
    };
    let exec_ranges: Vec<std::ops::Range<u64>> = segs
        .iter()
        .filter(|s| s.executable)
        .map(|s| s.vaddr..s.vaddr.saturating_add(s.memsz))
        .collect();
    if exec_ranges.is_empty() {
        return out;
    }
    let is_code_ptr = |p: u32| -> bool {
        let p = u64::from(p);
        exec_ranges.iter().any(|r| r.contains(&p))
    };
    let stride = INDIRECT_OPD_TABLE_STRIDE as usize;

    let mut i = 0usize;
    while i + stride <= data.len() {
        let col1 = read_u32(data, i + 4);
        if !is_code_ptr(col1) {
            i += 4;
            continue;
        }
        let Some(start_addr) = pt_load_file_to_guest(data, i) else {
            i += 4;
            continue;
        };
        let mut rows = 1usize;
        let mut j = i + stride;
        while j + stride <= data.len() && is_code_ptr(read_u32(data, j + 4)) {
            rows += 1;
            j += stride;
        }
        if rows >= INDIRECT_OPD_TABLE_MIN_ROWS {
            out.push(IndirectOpdTable {
                guest_addr: start_addr,
                size: (rows as u64) * INDIRECT_OPD_TABLE_STRIDE,
            });
            i = j;
            continue;
        }
        i += stride;
    }
    out
}
