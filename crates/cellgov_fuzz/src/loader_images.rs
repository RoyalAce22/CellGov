//! Structure-aware ELF and PRX images for the loader fuzz targets: the
//! seed images a fuzz run starts from, and the decoder that turns a
//! fuzz byte stream into a near-valid image.
//!
//! Random bytes rarely pass a magic check, so a byte-level fuzzer that
//! starts from nothing spends its budget on the first four bytes. An
//! image decoded from the stream field by field keeps every header
//! consistent unless the stream says otherwise. The parser then
//! reaches its table walks and relocation arithmetic on most inputs.
//! [Padhye2019 p:331 s:2.2 Coverage-Guided Fuzzing]

use cellgov_ps3_abi::format::elf::{
    ELF64_RELA_SIZE, ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE, ET_EXEC, ET_PRX,
    EXPORT_ATTR_SYSTEM, EXPORT_ENTRY_MIN_SIZE, NID_MODULE_START, NID_MODULE_STOP,
    PRX_IMPORT_ENTRY_FIRMWARE_SIZE, PRX_IMPORT_ENTRY_MIN_SIZE, PRX_IMPORT_ENTRY_VAR_MIN_SIZE,
    PRX_LIB_INFO_SIZE, PRX_MODULE_INFO_NAME_LEN, PRX_PARAM_HEADER_MIN_SIZE, PRX_PARAM_MAGIC,
    PT_LOAD, PT_PRX_PARAM, PT_PRX_RELOC, PT_TLS, R_PPC64_ADDR32,
};
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;

/// Size of one export entry the seed images declare.
const ENTRY_SIZE: usize = EXPORT_ENTRY_MIN_SIZE as usize;
/// Longest module name the module info holds, without its NUL.
const MODULE_NAME_MAX: usize = PRX_MODULE_INFO_NAME_LEN - 1;
/// Longest segment payload the stream decoder builds.
const MAX_SEGMENT_BYTES: u32 = 1024;
/// Byte alignment of a file section the renderer starts.
const SECTION_ALIGN: usize = 16;

/// Fixed-width fields read from a fuzz byte stream; an exhausted stream reads as zero.
#[derive(Debug, Clone)]
pub struct FieldStream<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> FieldStream<'a> {
    /// Reads `data` from its first byte.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> [u8; 8] {
        let mut out = [0u8; 8];
        for slot in out.iter_mut().take(n) {
            if let Some(&b) = self.data.get(self.pos) {
                *slot = b;
                self.pos += 1;
            }
        }
        out
    }

    /// The next byte.
    pub fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }

    /// The next two bytes, little-endian.
    pub fn u16(&mut self) -> u16 {
        let b = self.take(2);
        u16::from_le_bytes([b[0], b[1]])
    }

    /// The next four bytes, little-endian.
    pub fn u32(&mut self) -> u32 {
        let b = self.take(4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    /// The next eight bytes, little-endian.
    pub fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8))
    }

    /// A value below `n`; zero when `n` is zero.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.u32() % n
        }
    }

    /// The next `len` bytes.
    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.u8()).collect()
    }

    /// A value from the set a byte-level fuzzer substitutes as interesting.
    /// [Padhye2019 p:331 s:2.2 Coverage-Guided Fuzzing]
    pub fn interesting_u64(&mut self) -> u64 {
        match self.below(8) {
            0 => 0,
            1 => u64::MAX,
            2 => 1 << 63,
            3 => i64::MAX as u64,
            4 => 1 << 32,
            5 => u64::from(u32::MAX),
            6 => u64::from(self.u8()),
            _ => 1u64 << (self.u8() % 64),
        }
    }

    /// True when no unread byte remains.
    pub fn is_exhausted(&self) -> bool {
        self.pos >= self.data.len()
    }
}

fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_u64(buf: &mut [u8], at: usize, value: u64) {
    buf[at..at + 8].copy_from_slice(&value.to_be_bytes());
}

fn align_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

/// Write an ELF64 big-endian header whose program-header table starts
/// right after it.
fn write_elf_header(buf: &mut [u8], e_type: u16, entry: u64, phentsize: u16, phnum: u16) {
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
    fn write(&self, buf: &mut [u8], at: usize) {
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
fn nops(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    while out.len() + 4 <= len {
        out.extend_from_slice(&PPC_NOP.to_be_bytes());
    }
    out.resize(len, 0);
    out
}

/// One exported library of a [`PrxImage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrxExportLibrary {
    /// Library name, NUL-terminated by the renderer.
    pub name: Vec<u8>,
    /// Exported function NIDs; each gets an OPD in the data segment.
    pub functions: Vec<u32>,
    /// Exported variable NIDs; each gets a four-byte cell.
    pub variables: Vec<u32>,
}

/// One imported module of a [`PrxImage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrxImportModule {
    /// Module name, NUL-terminated by the renderer.
    pub name: Vec<u8>,
    /// Imported function NIDs; each gets a GOT slot.
    pub functions: Vec<u32>,
    /// Imported variable NIDs; the renderer writes them only when
    /// `entry_size` covers the variable fields.
    pub variables: Vec<u32>,
    /// The entry's declared size byte.
    pub entry_size: u8,
}

/// One RELA entry of the `PT_PRX_RELOC` segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrxRelocation {
    /// `r_offset`, relative to the target segment.
    pub offset: u64,
    /// PT_LOAD index the patch lands in (`r_sym & 0xFF`).
    pub target_segment: u8,
    /// PT_LOAD index whose address the value adds (`(r_sym >> 8) & 0xFF`).
    pub value_segment: u8,
    /// `r_addend`.
    pub addend: i64,
    /// `r_type`.
    pub rtype: u32,
}

/// A PS3 module image: a text segment, a data segment, a relocation
/// segment, and an optional `PT_PRX_PARAM` header.
///
/// The data segment holds the module info, the export and import
/// tables, and their OPDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrxImage {
    /// `e_type`; [`ET_PRX`] for a module, [`ET_EXEC`] for the
    /// game-executable shape that locates its imports through
    /// `PT_PRX_PARAM`.
    pub e_type: u16,
    /// Module name; the renderer keeps the first `MODULE_NAME_MAX` bytes.
    pub module_name: Vec<u8>,
    /// TOC, relative to the data segment.
    pub toc_offset: u32,
    /// Text segment bytes, at guest address zero.
    pub text: Vec<u8>,
    /// Guest address of the data segment.
    pub data_vaddr: u32,
    /// Whether the system export entry with `module_start` and
    /// `module_stop` is present.
    pub system_entry: bool,
    /// Exported libraries after the system entry.
    pub libraries: Vec<PrxExportLibrary>,
    /// Imported modules.
    pub imports: Vec<PrxImportModule>,
    /// Whether a `PT_PRX_PARAM` segment also locates the imports.
    pub imports_via_param: bool,
    /// Whether a zero-sized `PT_LOAD` placeholder follows text and data,
    /// so the content loads sit at indices 0 and 2.
    pub placeholder_loads: bool,
    /// Relocation entries.
    pub relocations: Vec<PrxRelocation>,
    /// Bytes after the last section.
    pub trailer: Vec<u8>,
}

/// Grows the data segment one reservation at a time.
struct DataBuilder {
    buf: Vec<u8>,
    vaddr: u32,
}

impl DataBuilder {
    fn reserve(&mut self, len: usize, align: usize) -> usize {
        let start = align_up(self.buf.len(), align);
        self.buf.resize(start + len, 0);
        start
    }

    fn vaddr_of(&self, offset: usize) -> u32 {
        self.vaddr.wrapping_add(offset as u32)
    }

    fn put_cstring(&mut self, name: &[u8]) -> u32 {
        let at = self.reserve(name.len() + 1, 1);
        self.buf[at..at + name.len()].copy_from_slice(name);
        self.vaddr_of(at)
    }

    fn put_opd(&mut self, code: u32, toc: u32) -> u32 {
        let at = self.reserve(8, 8);
        put_u32(&mut self.buf, at, code);
        put_u32(&mut self.buf, at + 4, toc);
        self.vaddr_of(at)
    }

    fn put_words(&mut self, words: &[u32]) -> u32 {
        let at = self.reserve(words.len() * 4, 4);
        for (i, word) in words.iter().enumerate() {
            put_u32(&mut self.buf, at + i * 4, *word);
        }
        self.vaddr_of(at)
    }
}

impl PrxImage {
    /// Render the image to file bytes.
    pub fn render(&self) -> Vec<u8> {
        let toc = self.data_vaddr.wrapping_add(self.toc_offset);
        let mut data = DataBuilder {
            buf: vec![0u8; PRX_LIB_INFO_SIZE],
            vaddr: self.data_vaddr,
        };

        // Export entries are contiguous; their tables follow.
        let system = usize::from(self.system_entry);
        let export_count = system + self.libraries.len();
        let exports_off = data.reserve(export_count * ENTRY_SIZE, 4);
        let exports_end = data.vaddr_of(exports_off + export_count * ENTRY_SIZE);
        if self.system_entry {
            let start = data.put_opd(0x10, toc);
            let stop = data.put_opd(0x20, toc);
            let nids = data.put_words(&[NID_MODULE_START, NID_MODULE_STOP]);
            let stubs = data.put_words(&[start, stop]);
            let at = exports_off;
            data.buf[at] = ENTRY_SIZE as u8;
            put_u16(&mut data.buf, at + 4, EXPORT_ATTR_SYSTEM);
            put_u16(&mut data.buf, at + 6, 2);
            put_u32(&mut data.buf, at + 20, nids);
            put_u32(&mut data.buf, at + 24, stubs);
        }
        for (i, lib) in self.libraries.iter().enumerate() {
            let name = data.put_cstring(&lib.name);
            let mut nids: Vec<u32> = lib.functions.clone();
            nids.extend_from_slice(&lib.variables);
            let mut stubs = Vec::with_capacity(nids.len());
            for (f, _) in lib.functions.iter().enumerate() {
                stubs.push(data.put_opd(0x30 + 4 * f as u32, toc));
            }
            for _ in &lib.variables {
                let cell = data.reserve(4, 4);
                stubs.push(data.vaddr_of(cell));
            }
            let nid_table = data.put_words(&nids);
            let stub_table = data.put_words(&stubs);
            let at = exports_off + (system + i) * ENTRY_SIZE;
            data.buf[at] = ENTRY_SIZE as u8;
            put_u16(&mut data.buf, at + 4, 1);
            put_u16(&mut data.buf, at + 6, lib.functions.len() as u16);
            put_u16(&mut data.buf, at + 8, lib.variables.len() as u16);
            put_u32(&mut data.buf, at + 16, name);
            put_u32(&mut data.buf, at + 20, nid_table);
            put_u32(&mut data.buf, at + 24, stub_table);
        }

        // Import entries are contiguous too, each at its declared size.
        let import_bytes: usize = self
            .imports
            .iter()
            .map(|m| usize::from(m.entry_size.max(PRX_IMPORT_ENTRY_MIN_SIZE)))
            .sum();
        let imports_off = data.reserve(import_bytes, 4);
        let imports_start = data.vaddr_of(imports_off);
        let imports_end = data.vaddr_of(imports_off + import_bytes);
        let mut at = imports_off;
        for module in &self.imports {
            let size = module.entry_size.max(PRX_IMPORT_ENTRY_MIN_SIZE);
            let name = data.put_cstring(&module.name);
            let nids = data.put_words(&module.functions);
            let slots = data.put_words(&vec![0u32; module.functions.len()]);
            data.buf[at] = module.entry_size;
            put_u16(&mut data.buf, at + 6, module.functions.len() as u16);
            put_u32(&mut data.buf, at + 16, name);
            put_u32(&mut data.buf, at + 20, nids);
            put_u32(&mut data.buf, at + 24, slots);
            if size >= PRX_IMPORT_ENTRY_VAR_MIN_SIZE {
                let vnids = data.put_words(&module.variables);
                let vslots = data.put_words(&vec![0u32; module.variables.len()]);
                put_u16(&mut data.buf, at + 8, module.variables.len() as u16);
                put_u32(&mut data.buf, at + 28, vnids);
                put_u32(&mut data.buf, at + 32, vslots);
            }
            at += usize::from(size);
        }

        // Module info at data offset zero, which segment 0's p_paddr
        // names by file offset; it doubles as the library info the
        // import walk reads.
        let name_len = self.module_name.len().min(MODULE_NAME_MAX);
        data.buf[4..4 + name_len].copy_from_slice(&self.module_name[..name_len]);
        put_u16(&mut data.buf, 0, 0x0006);
        data.buf[2] = 1;
        data.buf[3] = 1;
        put_u32(&mut data.buf, 32, toc);
        let exports_start = data.vaddr_of(exports_off);
        put_u32(&mut data.buf, 36, exports_start);
        put_u32(&mut data.buf, 40, exports_end);
        put_u32(&mut data.buf, 44, imports_start);
        put_u32(&mut data.buf, 48, imports_end);

        // File layout: header, program headers, text, data, relocations,
        // the optional parameter header, then the trailer.
        let placeholders = usize::from(self.placeholder_loads) * 2;
        let phnum = placeholders + 3 + usize::from(self.imports_via_param);
        let text_off = align_up(ELF_HEADER_SIZE + phnum * ELF_PHENTSIZE, SECTION_ALIGN);
        let data_off = align_up(text_off + self.text.len(), SECTION_ALIGN);
        let reloc_off = align_up(data_off + data.buf.len(), 8);
        let reloc_len = self.relocations.len() * ELF64_RELA_SIZE;
        let param_off = align_up(reloc_off + reloc_len, 4);
        let param_len = usize::from(self.imports_via_param) * PRX_PARAM_HEADER_MIN_SIZE as usize;
        let mut out = vec![0u8; param_off + param_len];
        write_elf_header(
            &mut out,
            self.e_type,
            0x10,
            ELF_PHENTSIZE as u16,
            phnum as u16,
        );

        let placeholder = Phdr {
            p_type: PT_LOAD,
            flags: 0,
            offset: 0,
            vaddr: 0,
            paddr: 0,
            filesz: 0,
            memsz: 0,
            align: 0x10000,
        };
        let text = Phdr {
            p_type: PT_LOAD,
            flags: 0x5,
            offset: text_off as u64,
            vaddr: 0,
            paddr: data_off as u64,
            filesz: self.text.len() as u64,
            memsz: self.text.len() as u64,
            align: 0x10000,
        };
        let data_phdr = Phdr {
            p_type: PT_LOAD,
            flags: 0x6,
            offset: data_off as u64,
            vaddr: u64::from(self.data_vaddr),
            paddr: 0,
            filesz: data.buf.len() as u64,
            memsz: data.buf.len() as u64 + 0x100,
            align: 0x10000,
        };
        let reloc = Phdr {
            p_type: PT_PRX_RELOC,
            flags: 0,
            offset: reloc_off as u64,
            vaddr: 0,
            paddr: 0,
            filesz: reloc_len as u64,
            memsz: 0,
            align: 8,
        };
        // Text stays at index 0. The import walk reads segment 0's
        // p_paddr for the library info, so a placeholder there hides
        // the imports from it.
        let mut phdrs = Vec::with_capacity(phnum);
        phdrs.push(text);
        if self.placeholder_loads {
            phdrs.push(placeholder.clone());
        }
        phdrs.push(data_phdr);
        if self.placeholder_loads {
            phdrs.push(placeholder);
        }
        phdrs.push(reloc);
        if self.imports_via_param {
            phdrs.push(Phdr {
                p_type: PT_PRX_PARAM,
                flags: 0,
                offset: param_off as u64,
                vaddr: 0,
                paddr: 0,
                filesz: param_len as u64,
                memsz: 0,
                align: 4,
            });
        }
        for (i, phdr) in phdrs.iter().enumerate() {
            phdr.write(&mut out, ELF_HEADER_SIZE + i * ELF_PHENTSIZE);
        }

        out[text_off..text_off + self.text.len()].copy_from_slice(&self.text);
        out[data_off..data_off + data.buf.len()].copy_from_slice(&data.buf);
        for (i, r) in self.relocations.iter().enumerate() {
            let at = reloc_off + i * ELF64_RELA_SIZE;
            let sym = (u32::from(r.value_segment) << 8) | u32::from(r.target_segment);
            put_u64(&mut out, at, r.offset);
            put_u64(
                &mut out,
                at + 8,
                (u64::from(sym) << 32) | u64::from(r.rtype),
            );
            put_u64(&mut out, at + 16, r.addend as u64);
        }
        if self.imports_via_param {
            put_u32(&mut out, param_off, PRX_PARAM_HEADER_MIN_SIZE);
            put_u32(&mut out, param_off + 4, PRX_PARAM_MAGIC);
            put_u32(&mut out, param_off + 24, imports_start);
            put_u32(&mut out, param_off + 28, imports_end);
        }
        out.extend_from_slice(&self.trailer);
        out
    }

    /// Decode an image from a fuzz byte stream.
    pub fn from_stream(s: &mut FieldStream<'_>) -> Self {
        let e_type = match s.below(8) {
            0 => ET_EXEC,
            1 => s.u16(),
            _ => ET_PRX,
        };
        let module_name = stream_name(s, 32);
        let toc_offset = match s.below(3) {
            0 => 0x200,
            1 => s.u32(),
            _ => 0,
        };
        let text_len = 0x40 + 4 * s.below(64) as usize;
        let text = if s.below(2) == 0 {
            nops(text_len)
        } else {
            s.bytes(text_len)
        };
        let data_vaddr = match s.below(3) {
            0 => 0x1000,
            1 => s.u32(),
            _ => s.below(0x10000),
        };
        let system_entry = s.below(4) != 0;
        let library_count = s.below(4) as usize;
        let libraries = (0..library_count)
            .map(|_| PrxExportLibrary {
                name: stream_name(s, 16),
                functions: stream_words(s, 6),
                variables: stream_words(s, 3),
            })
            .collect();
        let import_count = s.below(4) as usize;
        let imports = (0..import_count)
            .map(|_| {
                let name = stream_name(s, 24);
                let functions = stream_words(s, 6);
                let variables = stream_words(s, 3);
                let entry_size = match s.below(4) {
                    0 => PRX_IMPORT_ENTRY_MIN_SIZE,
                    1 => s.u8(),
                    _ => PRX_IMPORT_ENTRY_FIRMWARE_SIZE,
                };
                PrxImportModule {
                    name,
                    functions,
                    variables,
                    entry_size,
                }
            })
            .collect();
        let imports_via_param = s.below(3) == 0;
        let placeholder_loads = s.below(3) == 0;
        let relocation_count = s.below(6) as usize;
        let relocations = (0..relocation_count)
            .map(|_| {
                let offset = match s.below(3) {
                    0 => 4 * u64::from(s.below(64)),
                    1 => s.interesting_u64(),
                    _ => u64::from(s.u32()),
                };
                let target_segment = if s.below(4) == 0 {
                    s.u8()
                } else {
                    s.below(4) as u8
                };
                let value_segment = match s.below(3) {
                    0 => 0,
                    1 => 0xFF,
                    _ => s.u8(),
                };
                let addend = if s.below(2) == 0 {
                    i64::from(s.u16())
                } else {
                    s.u64() as i64
                };
                let rtype = if s.below(3) == 2 {
                    s.u32()
                } else {
                    R_PPC64_ADDR32
                };
                PrxRelocation {
                    offset,
                    target_segment,
                    value_segment,
                    addend,
                    rtype,
                }
            })
            .collect();
        let trailer_len = s.below(32) as usize;
        Self {
            e_type,
            module_name,
            toc_offset,
            text,
            data_vaddr,
            system_entry,
            libraries,
            imports,
            imports_via_param,
            placeholder_loads,
            relocations,
            trailer: s.bytes(trailer_len),
        }
    }
}

/// A name of up to `max` bytes; usually printable, sometimes the raw
/// stream bytes so the parsers meet control bytes and an unterminated
/// tail.
fn stream_name(s: &mut FieldStream<'_>, max: u32) -> Vec<u8> {
    let len = s.below(max) as usize;
    let raw = s.bytes(len);
    if s.below(4) == 0 {
        raw
    } else {
        raw.into_iter().map(|b| b'a' + b % 26).collect()
    }
}

fn stream_words(s: &mut FieldStream<'_>, max: u32) -> Vec<u32> {
    let count = s.below(max) as usize;
    (0..count).map(|_| s.u32()).collect()
}

/// Apply up to three byte-level corruptions the stream selects.
pub fn corrupt(bytes: &mut Vec<u8>, s: &mut FieldStream<'_>) {
    let rounds = s.below(4);
    for _ in 0..rounds {
        let len = bytes.len();
        if len == 0 {
            return;
        }
        let pos = s.below(len as u32) as usize;
        match s.below(6) {
            0 => bytes.truncate(pos),
            1 => bytes[pos] ^= 1 << s.below(8),
            2 => bytes[pos] = [0, 0xFF, 0x7F, 0x80][s.below(4) as usize],
            3 => {
                let at = (pos & !3).min(len.saturating_sub(4));
                if at + 4 <= len {
                    put_u32(bytes, at, s.u32());
                }
            }
            4 => {
                let at = (pos & !7).min(len.saturating_sub(8));
                if at + 8 <= len {
                    put_u64(bytes, at, s.interesting_u64());
                }
            }
            _ => {
                let count = s.below(16) as usize;
                let insert = s.bytes(count);
                bytes.splice(pos..pos, insert);
            }
        }
    }
}

/// The image a fuzz byte stream describes: an executable or a module,
/// rendered and then corrupted as the stream says.
pub fn structured_image(data: &[u8]) -> Vec<u8> {
    let mut s = FieldStream::new(data);
    let mut bytes = if s.u8() & 1 == 0 {
        ExecImage::from_stream(&mut s).render()
    } else {
        PrxImage::from_stream(&mut s).render()
    };
    corrupt(&mut bytes, &mut s);
    bytes
}

/// A named seed image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    /// File stem of the seed's file under `fuzz/seeds/`.
    pub name: &'static str,
    /// The image bytes.
    pub bytes: Vec<u8>,
}

/// Guest address of the seed executables' text segment.
pub const SEED_TEXT_VADDR: u64 = 0x1_0000;
/// Guest address of the seed executables' data segment.
pub const SEED_DATA_VADDR: u64 = 0x2_0000;
/// TOC the seed OPDs carry.
pub const SEED_TOC: u32 = 0x3_0000;

fn exec_segment(vaddr: u64, executable: bool, bytes: Vec<u8>, memsz: u64) -> ImageSegment {
    ImageSegment {
        p_type: PT_LOAD,
        flags: if executable { 0x5 } else { 0x6 },
        vaddr,
        paddr: 0,
        bytes,
        memsz,
        align: 0x10000,
    }
}

fn baseline_prx() -> PrxImage {
    PrxImage {
        e_type: ET_PRX,
        module_name: b"fuzzmod".to_vec(),
        toc_offset: 0x200,
        text: nops(0x100),
        data_vaddr: 0x1000,
        system_entry: true,
        libraries: vec![PrxExportLibrary {
            name: b"fuzzlib".to_vec(),
            functions: vec![0xAAAA_AAAA, 0xBBBB_BBBB],
            variables: vec![0xCCCC_CCCC],
        }],
        imports: Vec::new(),
        imports_via_param: false,
        placeholder_loads: false,
        relocations: vec![
            PrxRelocation {
                offset: 0x50,
                target_segment: 0,
                value_segment: 0,
                addend: 0x80,
                rtype: R_PPC64_ADDR32,
            },
            // The module_start OPD's code word: the first eight-byte
            // reservation after the module info and the two export
            // entries.
            PrxRelocation {
                offset: align_up(PRX_LIB_INFO_SIZE + 2 * ENTRY_SIZE, 8) as u64,
                target_segment: 1,
                value_segment: 0,
                addend: 0x10,
                rtype: R_PPC64_ADDR32,
            },
        ],
        trailer: Vec::new(),
    }
}

fn import_modules() -> Vec<PrxImportModule> {
    vec![
        PrxImportModule {
            name: b"sysPrxForUser".to_vec(),
            functions: vec![0x2F85_C0EF, 0x8461_E528],
            variables: vec![0x1D1E_1F20],
            entry_size: PRX_IMPORT_ENTRY_FIRMWARE_SIZE,
        },
        PrxImportModule {
            name: b"cellGcmSys".to_vec(),
            functions: vec![0x15BA_E46B],
            variables: Vec::new(),
            entry_size: PRX_IMPORT_ENTRY_MIN_SIZE,
        },
    ]
}

/// Every seed image, in a fixed order.
pub fn seeds() -> Vec<Seed> {
    let mut opd = vec![0u8; 0x40];
    put_u32(&mut opd, 0, SEED_TEXT_VADDR as u32);
    put_u32(&mut opd, 4, SEED_TOC);
    put_u32(&mut opd, 8, SEED_TEXT_VADDR as u32 + 0x20);
    put_u32(&mut opd, 12, SEED_TOC);

    let text_only = ExecImage {
        e_type: ET_EXEC,
        entry: SEED_TEXT_VADDR,
        phentsize: ELF_PHENTSIZE as u16,
        segments: vec![exec_segment(SEED_TEXT_VADDR, true, nops(0x40), 0x40)],
        trailer: Vec::new(),
    };
    let entry_opd = ExecImage {
        e_type: ET_EXEC,
        entry: SEED_DATA_VADDR,
        phentsize: ELF_PHENTSIZE as u16,
        segments: vec![
            exec_segment(SEED_TEXT_VADDR, true, nops(0x40), 0x40),
            exec_segment(SEED_DATA_VADDR, false, opd.clone(), 0x40),
        ],
        trailer: Vec::new(),
    };
    let bss_tail = ExecImage {
        e_type: ET_EXEC,
        entry: SEED_DATA_VADDR,
        phentsize: ELF_PHENTSIZE as u16,
        segments: vec![
            exec_segment(SEED_TEXT_VADDR, true, nops(0x40), 0x40),
            exec_segment(SEED_DATA_VADDR, false, opd, 0x1000),
        ],
        trailer: Vec::new(),
    };

    let mut prx_imports = baseline_prx();
    prx_imports.imports = import_modules();

    let mut prx_placeholders = baseline_prx();
    prx_placeholders.placeholder_loads = true;
    for r in &mut prx_placeholders.relocations {
        r.target_segment *= 2;
        r.value_segment *= 2;
    }

    let mut prx_param = baseline_prx();
    prx_param.imports = import_modules();
    prx_param.imports_via_param = true;

    let mut prx_no_system = baseline_prx();
    prx_no_system.system_entry = false;
    prx_no_system.relocations.truncate(1);

    let mut exec_param = baseline_prx();
    exec_param.e_type = ET_EXEC;
    exec_param.module_name = b"fuzzgame".to_vec();
    exec_param.imports = import_modules();
    exec_param.imports_via_param = true;
    exec_param.relocations.clear();

    vec![
        Seed {
            name: "exec_text_only",
            bytes: text_only.render(),
        },
        Seed {
            name: "exec_entry_opd",
            bytes: entry_opd.render(),
        },
        Seed {
            name: "exec_bss_tail",
            bytes: bss_tail.render(),
        },
        Seed {
            name: "exec_param_imports",
            bytes: exec_param.render(),
        },
        Seed {
            name: "prx_baseline",
            bytes: baseline_prx().render(),
        },
        Seed {
            name: "prx_imports",
            bytes: prx_imports.render(),
        },
        Seed {
            name: "prx_placeholders",
            bytes: prx_placeholders.render(),
        },
        Seed {
            name: "prx_param_imports",
            bytes: prx_param.render(),
        },
        Seed {
            name: "prx_no_system_entry",
            bytes: prx_no_system.render(),
        },
    ]
}

#[cfg(test)]
#[path = "tests/loader_images_tests.rs"]
mod tests;
