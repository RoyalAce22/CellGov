//! PS3 ELF / PRX / SPRX layout constants.
//!
//! Behaviour (the loader, the PRX import binder, the relocation
//! applicator) lives in `cellgov_ppu::loader` and `cellgov_ppu::sprx`;
//! this module is data only.
//!
//! Mixes general ELF values (PT_LOAD, R_PPC64_*) with PS3-specific
//! types (ET_PRX, PT_PRX_RELOC, NID_MODULE_*, SYS_PROCESS_PARAM_MAGIC).
//! CellGov only handles PS3 binaries, so co-locating them under the
//! PS3 ABI leaf is correct.

/// `\x7FELF` magic bytes at offset 0 of every ELF file.
pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// Size of the PS3 ELF header (e_phoff and friends are read from this).
pub const ELF_HEADER_SIZE: usize = 64;

/// `e_type` value for PS3 PRX modules (Sony-specific extension to ELF
/// `e_type`).
pub const ET_PRX: u16 = 0xFFA4;

/// `p_type` for normal loadable segments.
pub const PT_LOAD: u32 = 1;

/// `p_type` for the TLS template segment.
pub const PT_TLS: u32 = 7;

/// `p_type` for the PS3 PRX relocation table (carries the
/// `Elf64_Rela`-shaped entries the binder applies at load time).
pub const PT_PRX_RELOC: u32 = 0x7000_00A4;

/// `sh_type` for the symbol table section.
pub const SHT_SYMTAB: u32 = 2;

/// `sh_type` for the dynamic symbol table section.
pub const SHT_DYNSYM: u32 = 11;

/// `r_type` for `R_PPC64_ADDR32` (32-bit absolute).
pub const R_PPC64_ADDR32: u32 = 1;

/// `r_type` for `R_PPC64_ADDR16_LO` (low 16 bits).
pub const R_PPC64_ADDR16_LO: u32 = 4;

/// `r_type` for `R_PPC64_ADDR16_HI` (high 16 bits).
pub const R_PPC64_ADDR16_HI: u32 = 5;

/// `r_type` for `R_PPC64_ADDR16_HA` (high adjusted, sign-extension-aware).
pub const R_PPC64_ADDR16_HA: u32 = 6;

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
