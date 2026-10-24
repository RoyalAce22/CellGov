//! The PRX module model, its data-section builder, and its render and stream-decode pair.

use cellgov_ps3_abi::format::elf::{
    ELF64_RELA_SIZE, ELF_HEADER_SIZE, ELF_PHENTSIZE, ET_EXEC, ET_PRX, EXPORT_ATTR_SYSTEM,
    EXPORT_ENTRY_MIN_SIZE, NID_MODULE_START, NID_MODULE_STOP, PRX_IMPORT_ENTRY_FIRMWARE_SIZE,
    PRX_IMPORT_ENTRY_MIN_SIZE, PRX_IMPORT_ENTRY_VAR_MIN_SIZE, PRX_LIB_INFO_SIZE,
    PRX_MODULE_INFO_NAME_LEN, PRX_PARAM_HEADER_MIN_SIZE, PRX_PARAM_MAGIC, PT_LOAD, PT_PRX_PARAM,
    PT_PRX_RELOC, R_PPC64_ADDR32,
};

use std::ops::Range;

use super::elf::{align_up, nops, put_u16, put_u32, put_u64, write_elf_header, Phdr};
use super::stream::FieldStream;

/// Size of one export entry the seed images declare.
pub(super) const ENTRY_SIZE: usize = EXPORT_ENTRY_MIN_SIZE as usize;
/// Longest module name the module info holds, without its NUL.
const MODULE_NAME_MAX: usize = PRX_MODULE_INFO_NAME_LEN - 1;
/// Longest segment payload the stream decoder builds.
pub(super) const MAX_SEGMENT_BYTES: u32 = 1024;
/// Byte alignment of a file section the renderer starts.
const SECTION_ALIGN: usize = 16;

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
        // The reservation order (exports, imports, module info) fixes
        // every table address in the data segment.
        let exports = self.write_exports(&mut data, toc);
        let imports = self.write_imports(&mut data);
        self.write_module_info(&mut data, toc, &exports, &imports);

        let layout = self.layout(data.buf.len());
        let mut out = vec![0u8; layout.param_off + layout.param_len];
        write_elf_header(
            &mut out,
            self.e_type,
            0x10,
            ELF_PHENTSIZE as u16,
            layout.phnum as u16,
        );
        for (i, phdr) in self.program_headers(&layout).iter().enumerate() {
            phdr.write(&mut out, ELF_HEADER_SIZE + i * ELF_PHENTSIZE);
        }
        out[layout.text_off..layout.text_off + self.text.len()].copy_from_slice(&self.text);
        out[layout.data_off..layout.data_off + data.buf.len()].copy_from_slice(&data.buf);
        self.write_relocations(&mut out, layout.reloc_off);
        if self.imports_via_param {
            write_param_header(&mut out, layout.param_off, &imports);
        }
        out.extend_from_slice(&self.trailer);
        out
    }

    /// The export entries, contiguous, then each entry's OPDs and its
    /// NID and stub tables. Returns the guest addresses the entries span.
    fn write_exports(&self, data: &mut DataBuilder, toc: u32) -> Range<u32> {
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
        data.vaddr_of(exports_off)..exports_end
    }

    /// The import entries, contiguous and each at its declared size, then
    /// each entry's name, NID and slot tables. Returns the guest
    /// addresses the entries span.
    fn write_imports(&self, data: &mut DataBuilder) -> Range<u32> {
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
        imports_start..imports_end
    }

    /// The module info at data offset zero, which segment 0's p_paddr
    /// names by file offset; it doubles as the library info the import
    /// walk reads.
    fn write_module_info(
        &self,
        data: &mut DataBuilder,
        toc: u32,
        exports: &Range<u32>,
        imports: &Range<u32>,
    ) {
        let name_len = self.module_name.len().min(MODULE_NAME_MAX);
        data.buf[4..4 + name_len].copy_from_slice(&self.module_name[..name_len]);
        put_u16(&mut data.buf, 0, 0x0006);
        data.buf[2] = 1;
        data.buf[3] = 1;
        put_u32(&mut data.buf, 32, toc);
        put_u32(&mut data.buf, 36, exports.start);
        put_u32(&mut data.buf, 40, exports.end);
        put_u32(&mut data.buf, 44, imports.start);
        put_u32(&mut data.buf, 48, imports.end);
    }

    /// The file layout: header, program headers, text, data,
    /// relocations, the optional parameter header, then the trailer.
    fn layout(&self, data_len: usize) -> Layout {
        let placeholders = usize::from(self.placeholder_loads) * 2;
        let phnum = placeholders + 3 + usize::from(self.imports_via_param);
        let text_off = align_up(ELF_HEADER_SIZE + phnum * ELF_PHENTSIZE, SECTION_ALIGN);
        let data_off = align_up(text_off + self.text.len(), SECTION_ALIGN);
        let reloc_off = align_up(data_off + data_len, 8);
        let reloc_len = self.relocations.len() * ELF64_RELA_SIZE;
        let param_off = align_up(reloc_off + reloc_len, 4);
        let param_len = usize::from(self.imports_via_param) * PRX_PARAM_HEADER_MIN_SIZE as usize;
        Layout {
            phnum,
            text_off,
            data_off,
            data_len,
            reloc_off,
            reloc_len,
            param_off,
            param_len,
        }
    }

    fn program_headers(&self, layout: &Layout) -> Vec<Phdr> {
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
            offset: layout.text_off as u64,
            vaddr: 0,
            paddr: layout.data_off as u64,
            filesz: self.text.len() as u64,
            memsz: self.text.len() as u64,
            align: 0x10000,
        };
        let data_phdr = Phdr {
            p_type: PT_LOAD,
            flags: 0x6,
            offset: layout.data_off as u64,
            vaddr: u64::from(self.data_vaddr),
            paddr: 0,
            filesz: layout.data_len as u64,
            memsz: layout.data_len as u64 + 0x100,
            align: 0x10000,
        };
        let reloc = Phdr {
            p_type: PT_PRX_RELOC,
            flags: 0,
            offset: layout.reloc_off as u64,
            vaddr: 0,
            paddr: 0,
            filesz: layout.reloc_len as u64,
            memsz: 0,
            align: 8,
        };
        // Text stays at index 0. The import walk reads segment 0's
        // p_paddr for the library info, so a placeholder there hides
        // the imports from it.
        let mut phdrs = Vec::with_capacity(layout.phnum);
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
                offset: layout.param_off as u64,
                vaddr: 0,
                paddr: 0,
                filesz: layout.param_len as u64,
                memsz: 0,
                align: 4,
            });
        }
        phdrs
    }

    /// One RELA entry per relocation, from `reloc_off`.
    fn write_relocations(&self, out: &mut [u8], reloc_off: usize) {
        for (i, r) in self.relocations.iter().enumerate() {
            let at = reloc_off + i * ELF64_RELA_SIZE;
            let sym = (u32::from(r.value_segment) << 8) | u32::from(r.target_segment);
            put_u64(out, at, r.offset);
            put_u64(out, at + 8, (u64::from(sym) << 32) | u64::from(r.rtype));
            put_u64(out, at + 16, r.addend as u64);
        }
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

/// File offsets and lengths of a rendered image's sections.
struct Layout {
    phnum: usize,
    text_off: usize,
    data_off: usize,
    data_len: usize,
    reloc_off: usize,
    reloc_len: usize,
    param_off: usize,
    param_len: usize,
}

/// The `PT_PRX_PARAM` header, which locates the imports a second way.
fn write_param_header(out: &mut [u8], param_off: usize, imports: &Range<u32>) {
    put_u32(out, param_off, PRX_PARAM_HEADER_MIN_SIZE);
    put_u32(out, param_off + 4, PRX_PARAM_MAGIC);
    put_u32(out, param_off + 24, imports.start);
    put_u32(out, param_off + 28, imports.end);
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

#[cfg(test)]
#[path = "tests/prx_tests.rs"]
mod tests;
