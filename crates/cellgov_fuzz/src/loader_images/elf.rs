//! The ELF byte helpers, the header writer, and the executable image with its program headers.

use cellgov_ps3_abi::format::elf::{
    ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE, ET_EXEC, ET_PRX, PT_LOAD, PT_PRX_PARAM, PT_TLS,
};
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;

use super::prx::MAX_SEGMENT_BYTES;
use super::stream::FieldStream;

pub(super) fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn put_u64(buf: &mut [u8], at: usize, value: u64) {
    buf[at..at + 8].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn align_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

/// Write an ELF64 big-endian header whose program-header table starts
/// right after it.
pub(super) fn write_elf_header(
    buf: &mut [u8],
    e_type: u16,
    entry: u64,
    phentsize: u16,
    phnum: u16,
) {
    buf[0..4].copy_from_slice(&ELF_MAGIC);
    buf[4] = 2;
    buf[5] = 2;
    buf[6] = 1;
    put_u16(buf, 16, e_type);
    put_u16(buf, 18, 0x15);
    put_u32(buf, 20, 1);
    put_u64(buf, 24, entry);
    put_u64(buf, 32, ELF_HEADER_SIZE as u64);
    put_u16(buf, 52, ELF_HEADER_SIZE as u16);
    put_u16(buf, 54, phentsize);
    put_u16(buf, 56, phnum);
}

/// One ELF64 program header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phdr {
    /// `p_type`.
    pub p_type: u32,
    /// `p_flags`.
    pub flags: u32,
    /// `p_offset`.
    pub offset: u64,
    /// `p_vaddr`.
    pub vaddr: u64,
    /// `p_paddr`.
    pub paddr: u64,
    /// `p_filesz`.
    pub filesz: u64,
    /// `p_memsz`.
    pub memsz: u64,
    /// `p_align`.
    pub align: u64,
}

impl Phdr {
    pub(super) fn write(&self, buf: &mut [u8], at: usize) {
        put_u32(buf, at, self.p_type);
        put_u32(buf, at + 4, self.flags);
        put_u64(buf, at + 8, self.offset);
        put_u64(buf, at + 16, self.vaddr);
        put_u64(buf, at + 24, self.paddr);
        put_u64(buf, at + 32, self.filesz);
        put_u64(buf, at + 40, self.memsz);
        put_u64(buf, at + 48, self.align);
    }
}

/// One segment of an [`ExecImage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSegment {
    /// `p_type`; the seeds use `PT_LOAD`.
    pub p_type: u32,
    /// `p_flags`.
    pub flags: u32,
    /// Guest address of the first byte.
    pub vaddr: u64,
    /// `p_paddr`.
    pub paddr: u64,
    /// File bytes; `p_filesz` is their count.
    pub bytes: Vec<u8>,
    /// `p_memsz`.
    pub memsz: u64,
    /// `p_align`.
    pub align: u64,
}

/// A main-executable image: header, program headers, then each segment's
/// bytes at sequential file offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecImage {
    /// `e_type`.
    pub e_type: u16,
    /// `e_entry`.
    pub entry: u64,
    /// `e_phentsize`; only [`ELF_PHENTSIZE`] parses.
    pub phentsize: u16,
    /// Segments in program-header order.
    pub segments: Vec<ImageSegment>,
    /// Bytes after the last segment.
    pub trailer: Vec<u8>,
}

impl ExecImage {
    /// Render the image to file bytes.
    pub fn render(&self) -> Vec<u8> {
        let table = self.segments.len() * ELF_PHENTSIZE;
        let mut out = vec![0u8; ELF_HEADER_SIZE + table];
        write_elf_header(
            &mut out,
            self.e_type,
            self.entry,
            self.phentsize,
            self.segments.len() as u16,
        );
        for (i, seg) in self.segments.iter().enumerate() {
            let offset = out.len();
            out.extend_from_slice(&seg.bytes);
            Phdr {
                p_type: seg.p_type,
                flags: seg.flags,
                offset: offset as u64,
                vaddr: seg.vaddr,
                paddr: seg.paddr,
                filesz: seg.bytes.len() as u64,
                memsz: seg.memsz,
                align: seg.align,
            }
            .write(&mut out, ELF_HEADER_SIZE + i * ELF_PHENTSIZE);
        }
        out.extend_from_slice(&self.trailer);
        out
    }

    /// Decode an image from a fuzz byte stream.
    pub fn from_stream(s: &mut FieldStream<'_>) -> Self {
        let e_type = match s.below(4) {
            0 | 1 => ET_EXEC,
            2 => ET_PRX,
            _ => s.u16(),
        };
        let count = s.below(5) as usize;
        let mut segments = Vec::with_capacity(count);
        for i in 0..count {
            let p_type = match s.below(6) {
                0..=2 => PT_LOAD,
                3 => PT_TLS,
                4 => PT_PRX_PARAM,
                _ => s.u32(),
            };
            let flags = s.below(8);
            let vaddr = match s.below(4) {
                0 => 0x1_0000 * (i as u64 + 1),
                1 => s.interesting_u64(),
                2 => u64::from(s.u32()),
                _ => s.u64(),
            };
            let len = s.below(MAX_SEGMENT_BYTES + 1) as usize;
            let bytes = if s.below(2) == 0 {
                s.bytes(len)
            } else {
                nops(len)
            };
            let memsz = match s.below(4) {
                0 | 1 => len as u64,
                2 => len as u64 + u64::from(s.below(4096)),
                _ => s.interesting_u64(),
            };
            let paddr = if s.below(4) == 0 { s.u64() } else { 0 };
            let align = 1u64 << s.below(17);
            segments.push(ImageSegment {
                p_type,
                flags,
                vaddr,
                paddr,
                bytes,
                memsz,
                align,
            });
        }
        let entry = match s.below(3) {
            0 => segments.first().map_or(0, |seg| seg.vaddr),
            1 => u64::from(s.u32()),
            _ => s.interesting_u64(),
        };
        // An exhausted stream draws zero, so zero keeps the valid slot
        // size and a short input still reaches the segment walk.
        let phentsize = if s.below(8) == 7 {
            s.u16()
        } else {
            ELF_PHENTSIZE as u16
        };
        let trailer_len = s.below(64) as usize;
        Self {
            e_type,
            entry,
            phentsize,
            segments,
            trailer: s.bytes(trailer_len),
        }
    }
}

/// `len` bytes of PPU no-ops, the tail padded with zero when `len` is
/// not a multiple of four.
pub(super) fn nops(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    while out.len() + 4 <= len {
        out.extend_from_slice(&PPC_NOP.to_be_bytes());
    }
    out.resize(len, 0);
    out
}

#[cfg(test)]
#[path = "tests/elf_tests.rs"]
mod tests;
