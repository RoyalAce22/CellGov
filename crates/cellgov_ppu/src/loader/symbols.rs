//! Symbol lookup through the ELF symbol tables.

use cellgov_mem::be::{read_u16, read_u32, read_u64};
use cellgov_ps3_abi::format::elf::{
    ELF64_SHENT_SIZE, ELF_HEADER_SIZE, ELF_MAGIC, SHT_DYNSYM, SHT_SYMTAB,
};

/// Symbol address by name, or `None` if not found or the ELF has no
/// symbol table. Searches every `SHT_SYMTAB` and `SHT_DYNSYM` section
/// in order.
pub fn find_symbol(data: &[u8], name: &str) -> Option<u64> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC {
        return None;
    }
    let shoff = read_u64(data, 40) as usize;
    let shentsize = read_u16(data, 58) as usize;
    let shnum = read_u16(data, 60) as usize;
    // `sh_entsize` sits at section-header offset 56, so the validated
    // window has to span a full ELF64 section header even when
    // `e_shentsize` declares something narrower. Striding still uses
    // the declared value.
    let sh_window = shentsize.max(ELF64_SHENT_SIZE);

    for i in 0..shnum {
        let sh = shoff.checked_add(i.checked_mul(shentsize)?)?;
        if sh.checked_add(sh_window)? > data.len() {
            return None;
        }
        let sh_type = read_u32(data, sh + 4);
        if sh_type != SHT_SYMTAB && sh_type != SHT_DYNSYM {
            continue;
        }
        let sym_off = read_u64(data, sh + 24) as usize;
        let sym_size = read_u64(data, sh + 32) as usize;
        let sym_entsize = read_u64(data, sh + 56) as usize;
        let strtab_idx = read_u32(data, sh + 40) as usize;

        let Some(str_sh) = shoff.checked_add(strtab_idx.checked_mul(shentsize)?) else {
            continue;
        };
        if str_sh.checked_add(sh_window)? > data.len() {
            continue;
        }
        let str_off = read_u64(data, str_sh + 24) as usize;
        let str_size = read_u64(data, str_sh + 32) as usize;
        let Some(str_end) = str_off.checked_add(str_size) else {
            continue;
        };
        if str_end > data.len() {
            continue;
        }
        let strtab = &data[str_off..str_end];

        if sym_entsize == 0 {
            continue;
        }
        let count = sym_size / sym_entsize;
        for j in 0..count {
            // sh_offset and sh_size are 64-bit header fields: a hostile
            // pair near usize::MAX wraps this sum, and a wrapped index
            // passes the length check below while reading the wrong
            // bytes. Stop the section walk instead.
            let Some(entry) = j
                .checked_mul(sym_entsize)
                .and_then(|off| sym_off.checked_add(off))
            else {
                break;
            };
            // st_value lives at entry offset 8..16, so the validated
            // window never shrinks below 16 bytes however small
            // `sh_entsize` claims the entries are.
            match entry.checked_add(sym_entsize.max(16)) {
                Some(end) if end <= data.len() => {}
                _ => break,
            }
            let st_name = read_u32(data, entry) as usize;
            if st_name >= strtab.len() {
                continue;
            }
            let end = strtab[st_name..]
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(strtab.len() - st_name);
            let sym_name = &strtab[st_name..st_name + end];
            if sym_name == name.as_bytes() {
                return Some(read_u64(data, entry + 8));
            }
        }
    }
    None
}
