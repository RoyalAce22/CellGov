//! PS3 ELF / PRX / SPRX layout constants.
//!
//! Behaviour (the loader, the PRX import binder, the relocation
//! applicator) lives in `cellgov_ppu::loader` and `cellgov_ppu::sprx`;
//! this module is data only.

/// `\x7FELF` magic bytes at offset 0 of every ELF file.
pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// [`ELF_MAGIC`] as the big-endian word a header parser reads at offset 0.
pub const ELF_MAGIC_U32: u32 = u32::from_be_bytes(ELF_MAGIC);

/// Size of the PS3 PPU ELF header (ELF64).
pub const ELF_HEADER_SIZE: usize = 64;

/// Size of the PS3 SPU ELF header (ELF32); SPU binaries are 32-bit big-endian.
pub const ELF32_HEADER_SIZE: usize = 52;

/// Size of one ELF32 program header entry.
pub const ELF32_PHDR_SIZE: usize = 32;

/// `e_entry` field offset in the ELF32 header.
pub const ELF32_E_ENTRY: usize = 24;

// Same container coupling as the `ELF64_*` block below: a reader that
// bounds-checks `ELF32_HEADER_SIZE` reads this field without a second
// check.
const _: () = assert!(ELF32_E_ENTRY + 4 <= ELF32_HEADER_SIZE);

/// `e_ident[EI_CLASS]` value for 64-bit ELF.
pub const ELFCLASS64: u8 = 2;

/// `e_ident[EI_DATA]` value for big-endian (MSB) ELF.
pub const ELFDATA2MSB: u8 = 2;

/// `e_ident[EI_VERSION]` and `e_version` value for the current ELF revision.
pub const EV_CURRENT: u8 = 1;

/// Byte index of `EI_CLASS` within `e_ident`.
pub const ELF_EI_CLASS: usize = 4;

/// Byte index of `EI_DATA` within `e_ident`.
pub const ELF_EI_DATA: usize = 5;

/// Byte index of `EI_VERSION` within `e_ident`.
pub const ELF_EI_VERSION: usize = 6;

/// `e_machine` field offset in the ELF64 header.
pub const ELF_E_MACHINE_OFFSET: usize = 18;

/// `e_phnum` value reserved by the PN_XNUM extension: the real
/// program-header count lives in section header 0's `sh_info`, so a
/// parser that only walks the header table cannot honour it.
pub const ELF_PN_XNUM: u16 = 0xFFFF;

/// `e_type` value for executable files.
pub const ET_EXEC: u16 = 2;

/// `e_machine` value for 64-bit PowerPC.
pub const EM_PPC64: u16 = 21;

/// `e_type` value for PS3 PRX modules (Sony-specific extension to ELF
/// `e_type`).
pub const ET_PRX: u16 = 0xFFA4;

/// `p_flags` bit: execute permission on a PT_LOAD segment.
pub const PF_X: u32 = 1;

/// `p_flags` bit: write permission on a PT_LOAD segment.
pub const PF_W: u32 = 2;

/// `p_flags` bit: read permission on a PT_LOAD segment.
pub const PF_R: u32 = 4;

/// `p_type` for PS3 process-param segments (`PT_PROC_PARAM`,
/// 0x60000001 -- Sony PT_LOOS-range extension).
pub const PT_PROC_PARAM: u32 = 0x6000_0001;

/// Bytes covered by the eight `sys_process_param_t` fields the loader
/// reads, `size` through `ppc_seg`.
///
/// A record's own leading `size` field is its extent, and a title may
/// declare more than these 32 bytes. The eight 32-bit fields are the
/// whole record a toolchain emits. The loader warns about a record
/// that declares less, then reads the fields it covers.
pub const PROC_PARAM_SIZE: u64 = 32;

/// `p_type` for normal loadable segments.
pub const PT_LOAD: u32 = 1;

/// `p_type` for the TLS template segment.
pub const PT_TLS: u32 = 7;

/// `p_type` for the PS3 PRX relocation table (carries the
/// `Elf64_Rela`-shaped entries the binder applies at load time).
pub const PT_PRX_RELOC: u32 = 0x7000_00A4;

/// `p_type` for the PS3 `PrxParamHeader` segment (carries the
/// exports / imports table pointers).
pub const PT_PRX_PARAM: u32 = 0x6000_0002;

/// Magic value at offset +4 of the `PrxParamHeader` struct inside a
/// PT_PRX_PARAM segment. Rejects unrelated PT_LOOS payloads before
/// any table-pointer dereference.
pub const PRX_PARAM_MAGIC: u32 = 0x1b43_4cec;

/// ELF64 RELA entry size in bytes (`sizeof Elf64_Rela`: `r_offset`
/// u64 + `r_info` u64 + `r_addend` i64).
pub const ELF64_RELA_SIZE: usize = 24;

/// `sh_type` for the symbol table section.
pub const SHT_SYMTAB: u32 = 2;

/// `sh_type` for the dynamic symbol table section.
pub const SHT_DYNSYM: u32 = 11;

/// `sh_type` for sections holding program bits (code, rodata, data).
pub const SHT_PROGBITS: u32 = 1;

/// `sh_flags` bit: section occupies a runtime memory range.
pub const SHF_ALLOC: u64 = 0x2;

/// `sh_flags` bit: section contains machine instructions.
pub const SHF_EXECINSTR: u64 = 0x4;

/// ELF64 section-header-entry size (`e_shentsize`).
pub const ELF64_SHENT_SIZE: usize = 64;

/// Byte offset of `e_shoff` inside the ELF64 header.
pub const ELF64_E_SHOFF: usize = 40;

/// Byte offset of `e_shentsize` inside the ELF64 header.
pub const ELF64_E_SHENTSIZE: usize = 58;

/// Byte offset of `e_shnum` inside the ELF64 header.
pub const ELF64_E_SHNUM: usize = 60;

/// Byte offset of `e_shstrndx` (`u16`, section-header index of
/// `.shstrtab`) inside the ELF64 header.
pub const ELF64_E_SHSTRNDX: usize = 62;

// Byte offsets of the ELF64 section-header fields within one
// section-header-table entry: ELF64 spec Figure 1-9.

/// Byte offset of `sh_type` within an ELF64 section-header entry.
pub const ELF64_SH_TYPE: usize = 4;
/// Byte offset of `sh_name` (u32, strtab index) within an ELF64
/// section-header entry.
pub const ELF64_SH_NAME: usize = 0;
/// Byte offset of `sh_flags` within an ELF64 section-header entry.
pub const ELF64_SH_FLAGS: usize = 8;
/// Byte offset of `sh_offset` within an ELF64 section-header entry.
pub const ELF64_SH_OFFSET: usize = 24;
/// Byte offset of `sh_size` within an ELF64 section-header entry.
pub const ELF64_SH_SIZE: usize = 32;

/// `e_shstrndx` value indicating "no section name string table".
pub const SHN_UNDEF: u16 = 0;

/// `sh_type` for the string-table section (`.shstrtab`, `.strtab`).
pub const SHT_STRTAB: u32 = 3;

// Downstream readers (e.g. `cellgov_ppu::prescan::sections`) rely on
// every `ELF64_*` field fitting inside its container, so one runtime
// bounds check on the container size (`elf_data.len() >=
// ELF_HEADER_SIZE` or `shentsize >= ELF64_SHENT_SIZE`) covers every
// per-field read below.
const _: () = assert!(ELF64_E_SHOFF + 8 <= ELF_HEADER_SIZE);
const _: () = assert!(ELF64_E_SHENTSIZE + 2 <= ELF_HEADER_SIZE);
const _: () = assert!(ELF64_E_SHNUM + 2 <= ELF_HEADER_SIZE);
const _: () = assert!(ELF64_E_SHSTRNDX + 2 <= ELF_HEADER_SIZE);
const _: () = assert!(ELF64_SH_NAME + 4 <= ELF64_SHENT_SIZE);
const _: () = assert!(ELF64_SH_TYPE + 4 <= ELF64_SHENT_SIZE);
const _: () = assert!(ELF64_SH_FLAGS + 8 <= ELF64_SHENT_SIZE);
const _: () = assert!(ELF64_SH_OFFSET + 8 <= ELF64_SHENT_SIZE);
const _: () = assert!(ELF64_SH_SIZE + 8 <= ELF64_SHENT_SIZE);

/// The packed function descriptor a PS3 `.opd` entry holds.
///
/// `code@0` (u32), `toc@4` (u32), for a
/// [`SIZE`](function_descriptor::SIZE) of 8 bytes. A PPC64 ELFv1
/// descriptor is three doublewords. A PS3 effective address is 32
/// bits, so a PS3 entry packs two words.
///
/// The PPU loader dereferences `e_entry` as this pair. A guest
/// reaches the same pair through the `entry` word of a
/// [`thread_param`] block.
///
/// [`thread_param`]: crate::lv2::ppu_thread::thread_param
// No public document states the packing.
pub mod function_descriptor {
    /// Gives the descriptor size in bytes.
    pub const SIZE: usize = 8;

    /// `code` -- the guest address of the entry point's first
    /// instruction.
    pub const CODE_OFFSET: usize = 0;
    /// `toc` -- the r2 value the entry point runs under.
    pub const TOC_OFFSET: usize = 4;

    // Container coupling: a reader that bounds-checks `SIZE` reads
    // both words without a second check.
    const _: () = assert!(CODE_OFFSET + core::mem::size_of::<u32>() <= SIZE);
    const _: () = assert!(TOC_OFFSET + core::mem::size_of::<u32>() <= SIZE);

    /// Encodes one packed `.opd` entry as big-endian words.
    #[inline]
    pub const fn encode_ps3_packed_opd(code_addr: u32, toc: u32) -> [u8; SIZE] {
        let code_b = code_addr.to_be_bytes();
        let toc_b = toc.to_be_bytes();
        let mut out = [0u8; SIZE];
        let mut i = 0;
        while i < code_b.len() {
            out[CODE_OFFSET + i] = code_b[i];
            out[TOC_OFFSET + i] = toc_b[i];
            i += 1;
        }
        out
    }

    // `encode_ps3_packed_opd` writes both words from these offsets.
    const _: () = assert!(CODE_OFFSET + core::mem::size_of::<u32>() <= TOC_OFFSET);
}

#[cfg(test)]
#[path = "tests/elf_tests.rs"]
mod tests;

/// The three-doubleword PPC64 ELFv1 function descriptor that LV2 uses.
///
/// The packed [`function_descriptor`] form instead stores two 32-bit words.
pub mod ppc64_function_descriptor {
    // Retail LV2 inspection: every extracted kernel descriptor uses
    // three big-endian u64 words in this order.
    /// Bytes one descriptor occupies.
    pub const SIZE: usize = 24;
    /// Gives the byte offset of the code address.
    pub const CODE_OFFSET: usize = 0;
    /// Gives the byte offset of the TOC address.
    pub const TOC_OFFSET: usize = 8;
    /// Gives the byte offset of the environment word.
    pub const ENV_OFFSET: usize = 16;

    const _: () = assert!(CODE_OFFSET + core::mem::size_of::<u64>() <= SIZE);
    const _: () = assert!(TOC_OFFSET + core::mem::size_of::<u64>() <= SIZE);
    const _: () = assert!(ENV_OFFSET + core::mem::size_of::<u64>() <= SIZE);
}

/// `r_type` for `R_PPC64_ADDR32` (32-bit absolute).
pub const R_PPC64_ADDR32: u32 = 1;

/// `r_type` for `R_PPC64_ADDR16_LO` (low 16 bits).
pub const R_PPC64_ADDR16_LO: u32 = 4;

/// `r_type` for `R_PPC64_ADDR16_HI` (high 16 bits).
pub const R_PPC64_ADDR16_HI: u32 = 5;

/// `r_type` for `R_PPC64_ADDR16_HA` (high adjusted, sign-extension-aware).
pub const R_PPC64_ADDR16_HA: u32 = 6;

/// `r_type` for `R_PPC64_ADDR64` (64-bit absolute).
pub const R_PPC64_ADDR64: u32 = 38;

/// `r_type` for `R_PPC64_REL24` (24-bit pc-relative branch).
pub const R_PPC64_REL24: u32 = 10;

/// `r_type` for `R_PPC64_ADDR16_LO_DS` (low 16 bits, DS-form, low 2 bits preserved).
pub const R_PPC64_ADDR16_LO_DS: u32 = 57;

/// Value-segment index in a PS3 PRX RELA entry that names no segment.
///
/// The addend is then a whole address.
pub const PRX_RELOC_NO_VALUE_SEGMENT: u32 = 0xFF;

/// PS3 module entrypoint NID (`module_start` noname-export).
pub const NID_MODULE_START: u32 = 0xbc9a_0086;

/// PS3 module exitpoint NID (`module_stop` noname-export).
pub const NID_MODULE_STOP: u32 = 0xab77_9874;

/// Minimum size in bytes of a PRX `libent` export entry (28 bytes).
pub const EXPORT_ENTRY_MIN_SIZE: u8 = 0x1C;

/// `attr` bit for system-class exports (Sony's first-party libraries
/// emit this; user-mode PRXs do not).
pub const EXPORT_ATTR_SYSTEM: u16 = 0x8000;

/// Magic word at the start of the `sys_process_param_t` struct that
/// the loader looks up via the `.sys_proc_param` section. Every PS3
/// title's process-param block starts with this 32-bit BE word.
pub const SYS_PROCESS_PARAM_MAGIC: u32 = 0x13bc_c5f6;

/// Sentinel `sys_process_get_sdk_version` returns when no
/// `sys_process_param_t` segment is present (PSL1GHT homebrew with no
/// recorded SDK build). The process-param vocabulary reserves this
/// value as the unknown-SDK marker, so a title may also declare it.
/// Retail titles always carry a real version here, and cellSysutil's
/// SDK-keyed init dispatcher takes a different branch on the sentinel.
pub const SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN: u32 = 0xFFFF_FFFF;

/// `version` field of `sys_process_param_t` for SDK 3.30 and later.
///
/// The field is a namespace of its own, unrelated to `sdk_version`.
/// Every firmware executable in the 4.92 image carries this value
/// alongside `sdk_version` 0x00492000. A 2.50-era title carries
/// 0x00009000 alongside `sdk_version` 0x00250001. Each member of the
/// `SYS_PROCESS_PARAM_VERSION_*` space takes the name of the SDK
/// release that first emitted it.
pub const SYS_PROCESS_PARAM_VERSION_330_0: u32 = 0x0033_0000;

/// `e_phoff` field offset in the ELF64 header.
pub const ELF_PHOFF_OFFSET: usize = 32;

/// `e_phentsize` field offset in the ELF64 header.
pub const ELF_PHENTSIZE_OFFSET: usize = 54;

/// `e_phnum` field offset in the ELF64 header.
pub const ELF_PHNUM_OFFSET: usize = 56;

/// Size of one ELF64 program header entry.
pub const ELF_PHENTSIZE: usize = 56;

/// `p_offset` field offset within an ELF64 program header.
pub const PHDR_P_OFFSET_OFFSET: usize = 8;

/// `p_vaddr` field offset within an ELF64 program header.
pub const PHDR_P_VADDR_OFFSET: usize = 16;

/// `p_paddr` field offset within an ELF64 program header. PS3
/// firmware PRXs repurpose this field on segment 0 to point at a
/// `ppu_prx_library_info` struct; see [`PRX_LIB_INFO_SIZE`] and
/// friends.
pub const PHDR_P_PADDR_OFFSET: usize = 24;

/// `p_filesz` field offset within an ELF64 program header.
pub const PHDR_P_FILESZ_OFFSET: usize = 32;

/// `p_memsz` field offset within an ELF64 program header. Exceeds
/// `p_filesz` by the segment's zero-fill (`.bss`) tail.
pub const PHDR_P_MEMSZ_OFFSET: usize = 40;

// Same coupling contract as the `ELF64_*` block above, for the
// program-header field offsets: a reader that has bounds-checked the
// container (`ELF_HEADER_SIZE` bytes of header, `ELF_PHENTSIZE` bytes
// per phdr slot) can read any field below without a second check.
const _: () = assert!(ELF_EI_CLASS < ELF_HEADER_SIZE);
const _: () = assert!(ELF_EI_DATA < ELF_HEADER_SIZE);
const _: () = assert!(ELF_EI_VERSION < ELF_HEADER_SIZE);
const _: () = assert!(ELF_E_MACHINE_OFFSET + 2 <= ELF_HEADER_SIZE);
const _: () = assert!(ELF_PHOFF_OFFSET + 8 <= ELF_HEADER_SIZE);
const _: () = assert!(ELF_PHENTSIZE_OFFSET + 2 <= ELF_HEADER_SIZE);
const _: () = assert!(ELF_PHNUM_OFFSET + 2 <= ELF_HEADER_SIZE);
const _: () = assert!(PHDR_P_OFFSET_OFFSET + 8 <= ELF_PHENTSIZE);
const _: () = assert!(PHDR_P_VADDR_OFFSET + 8 <= ELF_PHENTSIZE);
const _: () = assert!(PHDR_P_PADDR_OFFSET + 8 <= ELF_PHENTSIZE);
const _: () = assert!(PHDR_P_FILESZ_OFFSET + 8 <= ELF_PHENTSIZE);
const _: () = assert!(PHDR_P_MEMSZ_OFFSET + 8 <= ELF_PHENTSIZE);

/// Offset of the `header_size` u32 field in `PrxParamHeader`.
pub const PRX_PARAM_HEADER_SIZE_OFFSET: usize = 0;

/// Offset of the `magic` u32 field in `PrxParamHeader`.
pub const PRX_PARAM_MAGIC_OFFSET: usize = 4;

/// Offset of the `imports_table_start` u32 field in `PrxParamHeader`.
pub const PRX_PARAM_IMPORTS_START_OFFSET: usize = 24;

/// Offset of the `imports_table_end` u32 field in `PrxParamHeader`.
pub const PRX_PARAM_IMPORTS_END_OFFSET: usize = 28;

/// Minimum `header_size` value accepted for a `PrxParamHeader`: the
/// imports table fields live at +24/+28, so a declared size below
/// 32 would let the parser read those offsets against unrelated
/// bytes.
pub const PRX_PARAM_HEADER_MIN_SIZE: u32 = 32;

// `PrxImportEntry` is the record a PS3 PRX carries once per imported
// module. The offsets below locate its module name, its imported NIDs
// and the GOT slots the binder patches. Every `.prx` in a retail
// `dev_flash/sys/external` carries this record.

/// Offset of the `size` byte (declared entry size) in
/// `PrxImportEntry`.
pub const PRX_IMPORT_SIZE_OFFSET: usize = 0;

/// Offset of the `num_func` u16 field in `PrxImportEntry`.
pub const PRX_IMPORT_NUM_FUNC_OFFSET: usize = 6;

/// Offset of the `num_var` u16 field (variable imports) in
/// `PrxImportEntry`. Read only when the declared entry size covers the
/// variable section.
pub const PRX_IMPORT_NUM_VAR_OFFSET: usize = 8;

/// Offset of the `name_ptr` u32 field in `PrxImportEntry`.
pub const PRX_IMPORT_NAME_PTR_OFFSET: usize = 16;

/// Offset of the `nids_ptr` u32 field in `PrxImportEntry`.
pub const PRX_IMPORT_NIDS_PTR_OFFSET: usize = 20;

/// Offset of the `addrs_ptr` u32 field (also called `stub_ptr`:
/// the address of the GOT slot table the binder patches) in
/// `PrxImportEntry`.
pub const PRX_IMPORT_STUB_PTR_OFFSET: usize = 24;

/// Offset of the `vnids_ptr` u32 field (imported VNIDs) in
/// `PrxImportEntry`. Only present when the declared entry size is at
/// least 32 bytes.
///
/// The installed firmware witnesses this field. Every import entry it
/// declares is 0x2c bytes, which covers the field. The `libadec`,
/// `libadec2`, `libadec_internal` and `libdmux` modules carry a
/// non-zero `num_var` against it.
pub const PRX_IMPORT_VNIDS_PTR_OFFSET: usize = 28;

/// Offset of the `vstubs_ptr` u32 field (variable slot table the
/// binder patches at boot) in `PrxImportEntry`. Only present when the
/// declared entry size is at least 36 bytes.
///
/// Witnessed on the same terms as [`PRX_IMPORT_VNIDS_PTR_OFFSET`].
pub const PRX_IMPORT_VSTUBS_PTR_OFFSET: usize = 32;

/// Minimum declared entry size for variable-import parsing to be
/// safe (covers through `vstubs_ptr` at +32). Entries smaller than
/// this are treated as function-only.
pub const PRX_IMPORT_ENTRY_VAR_MIN_SIZE: u8 = 36;

/// The entry size every installed firmware module declares, with the
/// variable fields and eight bytes past them.
pub const PRX_IMPORT_ENTRY_FIRMWARE_SIZE: u8 = 0x2C;

/// Smallest declared entry size a `PrxImportEntry` can carry and
/// still be parseable: an entry whose declared `size` byte is below
/// this is structurally corrupt, because its fields would not cover
/// the `addrs_ptr` field at +24. A conforming entry may declare more,
/// with fields past `vstubs` at +32 that this parser never reads.
pub const PRX_IMPORT_ENTRY_MIN_SIZE: u8 = 0x1C;

/// Cap on the length of a C string the PRX parser will accept when
/// dereferencing a name pointer; anything longer is rejected as
/// malformed rather than copied into an `ImportedModule`.
pub const PRX_NAME_MAX_LEN: usize = 256;

// `ppu_prx_library_info` is the record a firmware PRX points at from
// segment 0's `p_paddr` instead of shipping a `PT_PRX_PARAM` segment.
// The offsets below reach the import table of every
// `dev_flash/sys/external` module. The type name is inherited
// vocabulary, not a Sony name.

/// Offset of the `imports_start` u32 field in `ppu_prx_library_info`.
pub const PRX_LIB_INFO_IMPORTS_START_OFFSET: usize = 44;

/// Offset of the `imports_end` u32 field in `ppu_prx_library_info`.
pub const PRX_LIB_INFO_IMPORTS_END_OFFSET: usize = 48;

/// Size in bytes of one `ppu_prx_library_info` struct;
/// `sys_prx_module_info_t` shares the layout.
pub const PRX_LIB_INFO_SIZE: usize = 52;

/// Bytes of the `name` field at +4 of `sys_prx_module_info_t`, NUL
/// included.
pub const PRX_MODULE_INFO_NAME_LEN: usize = 28;
