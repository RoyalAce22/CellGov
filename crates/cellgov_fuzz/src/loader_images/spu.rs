//! The SPU image forms: an ELF32 SPU executable, and the job image with
//! no ELF header that loads as one block of local store.

use cellgov_ps3_abi::format::elf::{
    ELF32_E_ENTRY, ELF32_E_PHENTSIZE, ELF32_E_PHNUM, ELF32_E_PHOFF, ELF32_HEADER_SIZE,
    ELF32_PHDR_SIZE, ELF32_P_FLAGS, ELF_E_MACHINE_OFFSET, ELF_MAGIC, EM_SPU, ET_EXEC, PT_LOAD,
};
use cellgov_ps3_abi::hw::spu::SPU_LS_SIZE;

use super::elf::{put_u16, put_u32};
use super::stream::FieldStream;

/// Local-store address the job-image input form loads at.
pub const JOB_IMAGE_LS: u32 = 0x4000;
/// Offset of the job-image form's first instruction from its first byte.
pub const JOB_IMAGE_CODE_OFFSET: u32 = 0x30;
/// Most file bytes one segment the stream describes holds.
const MAX_SPU_SEGMENT_BYTES: u32 = 0x800;
/// Most segments a described segment list holds.
const MAX_LS_SEGMENTS: u32 = 4;
/// The top of local store, which the stream's addresses cluster around.
const LS_TOP: u32 = SPU_LS_SIZE as u32;

/// One program header of an [`SpuElfImage`] and its file bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuImageSegment {
    /// `p_type`.
    pub p_type: u32,
    /// Local-store address of the first byte.
    pub vaddr: u32,
    /// File bytes; `p_filesz` is their count.
    pub bytes: Vec<u8>,
    /// `p_memsz`.
    pub memsz: u32,
    /// `p_flags`.
    pub flags: u32,
}

/// An ELF32 big-endian SPU executable: header, program headers, then each
/// segment's bytes at sequential file offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuElfImage {
    /// `e_entry`.
    pub entry: u32,
    /// `e_phentsize`; only the ELF32 program-header size parses.
    pub phentsize: u16,
    /// Segments in program-header order.
    pub segments: Vec<SpuImageSegment>,
    /// Bytes after the last segment.
    pub trailer: Vec<u8>,
}

impl SpuElfImage {
    /// Render the image to file bytes.
    pub fn render(&self) -> Vec<u8> {
        let table = self.segments.len() * ELF32_PHDR_SIZE;
        let mut out = vec![0u8; ELF32_HEADER_SIZE + table];
        out[0..4].copy_from_slice(&ELF_MAGIC);
        out[4] = 1;
        out[5] = 2;
        out[6] = 1;
        put_u16(&mut out, 16, ET_EXEC);
        put_u16(&mut out, ELF_E_MACHINE_OFFSET, EM_SPU);
        put_u32(&mut out, 20, 1);
        put_u32(&mut out, ELF32_E_ENTRY, self.entry);
        put_u32(&mut out, ELF32_E_PHOFF, ELF32_HEADER_SIZE as u32);
        put_u16(&mut out, 40, ELF32_HEADER_SIZE as u16);
        put_u16(&mut out, ELF32_E_PHENTSIZE, self.phentsize);
        put_u16(&mut out, ELF32_E_PHNUM, self.segments.len() as u16);
        for (i, seg) in self.segments.iter().enumerate() {
            let offset = out.len() as u32;
            out.extend_from_slice(&seg.bytes);
            let at = ELF32_HEADER_SIZE + i * ELF32_PHDR_SIZE;
            put_u32(&mut out, at, seg.p_type);
            put_u32(&mut out, at + 4, offset);
            put_u32(&mut out, at + 8, seg.vaddr);
            put_u32(&mut out, at + 16, seg.bytes.len() as u32);
            put_u32(&mut out, at + 20, seg.memsz);
            put_u32(&mut out, at + ELF32_P_FLAGS, seg.flags);
            put_u32(&mut out, at + 28, 0x80);
        }
        out.extend_from_slice(&self.trailer);
        out
    }

    /// Decode an image from a fuzz byte stream.
    pub fn from_stream(s: &mut FieldStream<'_>) -> Self {
        let count = s.below(4) as usize;
        let mut segments = Vec::with_capacity(count);
        for i in 0..count {
            let p_type = if s.below(5) == 4 { s.u32() } else { PT_LOAD };
            let vaddr = ls_address(s, 0x1000 * i as u32);
            let len = s.below(MAX_SPU_SEGMENT_BYTES + 1);
            let bytes = if s.below(2) == 0 {
                s.bytes(len as usize)
            } else {
                vec![0; len as usize]
            };
            let memsz = match s.below(4) {
                0 | 1 => len,
                2 => len + s.below(0x4000),
                _ => s.u32(),
            };
            segments.push(SpuImageSegment {
                p_type,
                vaddr,
                bytes,
                memsz,
                flags: s.below(8),
            });
        }
        let entry = match s.below(3) {
            0 => segments.first().map_or(0, |seg| seg.vaddr),
            1 => s.u32(),
            _ => LS_TOP - 4 + s.below(8),
        };
        // An exhausted stream draws zero, so zero keeps the valid slot
        // size and a short input still reaches the segment walk.
        let phentsize = if s.below(8) == 7 {
            s.u16()
        } else {
            ELF32_PHDR_SIZE as u16
        };
        let trailer_len = s.below(64) as usize;
        Self {
            entry,
            phentsize,
            segments,
            trailer: s.bytes(trailer_len),
        }
    }
}

/// A local-store address: `usual`, one near the end of local store, or
/// any word.
fn ls_address(s: &mut FieldStream<'_>, usual: u32) -> u32 {
    match s.below(4) {
        0 | 1 => usual,
        2 => LS_TOP - s.below(0x1000),
        _ => s.u32(),
    }
}

/// The SPU ELF image a fuzz byte stream describes, rendered and then
/// corrupted as the stream says.
pub fn spu_elf_image(data: &[u8]) -> Vec<u8> {
    let mut s = FieldStream::new(data);
    let mut bytes = SpuElfImage::from_stream(&mut s).render();
    super::corrupt(&mut bytes, &mut s);
    bytes
}

/// The arguments of one `load_ls_segments` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LsSegments {
    /// `(local-store address, bytes)` pairs, in load order.
    pub segments: Vec<(u32, Vec<u8>)>,
    /// The entry point.
    pub entry: u32,
}

impl LsSegments {
    /// A job image: every byte of `data` at [`JOB_IMAGE_LS`], entered at
    /// its first instruction.
    pub fn job_image(data: &[u8]) -> Self {
        Self {
            segments: vec![(JOB_IMAGE_LS, data.to_vec())],
            entry: JOB_IMAGE_LS + JOB_IMAGE_CODE_OFFSET,
        }
    }

    /// A segment list and entry point the stream describes.
    pub fn from_stream(s: &mut FieldStream<'_>) -> Self {
        let count = s.below(MAX_LS_SEGMENTS + 1) as usize;
        let mut segments = Vec::with_capacity(count);
        for i in 0..count {
            let at = ls_address(s, JOB_IMAGE_LS * i as u32);
            let len = s.below(MAX_SPU_SEGMENT_BYTES + 1) as usize;
            segments.push((at, s.bytes(len)));
        }
        let entry = ls_address(s, segments.first().map_or(0, |seg| seg.0));
        Self { segments, entry }
    }

    /// The call `data` stands for: an even first byte, or none, makes the
    /// whole input a job image; an odd one makes the rest a described list.
    pub fn from_bytes(data: &[u8]) -> Self {
        match data.split_first() {
            Some((first, rest)) if first & 1 == 1 => Self::from_stream(&mut FieldStream::new(rest)),
            _ => Self::job_image(data),
        }
    }
}

/// The same input read as the other `load_ls_segments` form: its first
/// byte's low bit flipped, or a lone odd byte for an empty input.
pub fn other_ls_form(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    match out.first_mut() {
        Some(first) => *first ^= 1,
        None => out.push(1),
    }
    out
}

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;
