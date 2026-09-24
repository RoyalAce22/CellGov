//! The module identity a PPU object carries.

use super::*;

/// Minimal ELF64-BE header carrying `e_type`, enough for `parse_prx`
/// to reach its e_type check.
fn elf64_be_of_type(e_type: u16) -> Vec<u8> {
    let mut data = vec![0u8; 128];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2; // ELFCLASS64
    data[5] = 2; // ELFDATA2MSB
    data[16..18].copy_from_slice(&e_type.to_be_bytes());
    data
}

#[test]
fn a_title_executable_has_no_module_info_and_that_is_not_a_refusal() {
    let eboot = elf64_be_of_type(ET_EXEC);
    assert!(module_identity(&eboot)
        .expect("ET_EXEC is the EBOOT case")
        .is_none());
}

#[test]
fn a_container_that_is_neither_prx_nor_exec_is_named_not_read_as_an_eboot() {
    // ET_REL: no sys_prx_module_info_t and not a title executable
    // either, so printing its import table as authoritative without a
    // word is the failure this arm exists to prevent.
    let rel = elf64_be_of_type(1);
    assert!(matches!(
        module_identity(&rel),
        Err(PrxParseError::NotPrx(1))
    ));
}

#[test]
fn a_prx_type_answers_what_parse_prx_answers() {
    // A PRX-typed header with no segments is refused by parse_prx, and
    // module_identity passes that refusal through rather than reading
    // the file as an executable.
    let prx = elf64_be_of_type(ET_PRX);
    assert_eq!(
        module_identity(&prx).map(|p| p.map(|p| p.name)),
        parse_prx(&prx).map(|p| Some(p.name))
    );
    assert!(module_identity(&prx).is_err());
}
