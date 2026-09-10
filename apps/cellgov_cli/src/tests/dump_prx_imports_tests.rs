//! Name-column formatting, source classification, and stub hints.

use super::*;

// -- fit_name_column ---------------------------------------------------

#[test]
fn fit_name_column_passes_short_unchanged() {
    assert_eq!(fit_name_column("cellFsOpen"), "cellFsOpen");
}

#[test]
fn fit_name_column_passes_exact_width_unchanged() {
    let exact = "x".repeat(NAME_COLUMN_WIDTH);
    assert_eq!(fit_name_column(&exact), exact);
    assert_eq!(fit_name_column(&exact).chars().count(), NAME_COLUMN_WIDTH);
}

#[test]
fn fit_name_column_truncates_at_width_plus_one() {
    let plus_one = "x".repeat(NAME_COLUMN_WIDTH + 1);
    let fit = fit_name_column(&plus_one);
    assert_eq!(fit.chars().count(), NAME_COLUMN_WIDTH);
    assert!(fit.ends_with("..."));
}

#[test]
fn fit_name_column_truncates_overlong_with_ellipsis() {
    let long = "x".repeat(NAME_COLUMN_WIDTH + 10);
    let fit = fit_name_column(&long);
    assert_eq!(fit.chars().count(), NAME_COLUMN_WIDTH);
    assert!(fit.ends_with("..."));
}

// -- classify_source ---------------------------------------------------

fn pad_with_elf_magic(len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[0..4].copy_from_slice(&ELF_MAGIC);
    v
}

fn pad_with_sce_magic(len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[0..4].copy_from_slice(&SCE_MAGIC);
    v
}

#[test]
fn classify_source_routes_elf_magic_to_elf() {
    let buf = pad_with_elf_magic(cellgov_ps3_abi::format::elf::ELF_HEADER_SIZE);
    assert_eq!(classify_source(&buf), Ok(SourceKind::Elf));
}

#[test]
fn classify_source_routes_sce_magic_to_sce_wrapped() {
    let buf = pad_with_sce_magic(cellgov_ps3_abi::format::elf::ELF_HEADER_SIZE);
    assert_eq!(classify_source(&buf), Ok(SourceKind::SceWrapped));
}

#[test]
fn classify_source_rejects_short_file_even_with_valid_magic() {
    let buf = ELF_MAGIC.to_vec();
    assert_eq!(classify_source(&buf), Err(LoadError::TooSmall { len: 4 }));
}

#[test]
fn classify_source_rejects_bad_magic() {
    let mut buf = vec![0u8; cellgov_ps3_abi::format::elf::ELF_HEADER_SIZE];
    buf[0..4].copy_from_slice(b"BAD!");
    assert_eq!(
        classify_source(&buf),
        Err(LoadError::BadMagic { magic: *b"BAD!" })
    );
}

// -- nearest_stub_hint -------------------------------------------------

fn module(name: &str, stubs: &[(u32, u32)]) -> cellgov_ppu::prx::ImportedModule {
    cellgov_ppu::prx::ImportedModule {
        name: name.to_string(),
        functions: stubs
            .iter()
            .map(|&(nid, stub)| cellgov_ppu::prx::ImportedFunction {
                nid,
                stub_addr: stub,
            })
            .collect(),
        variables: Vec::new(),
    }
}

#[test]
fn nearest_stub_hint_finds_closest_across_modules() {
    let mods = [
        module("A", &[(0x1, 0x009b_f000), (0x2, 0x009b_f100)]),
        module("B", &[(0x3, 0x009b_ff00)]),
    ];
    let scope: Vec<&_> = mods.iter().collect();
    let hint = nearest_stub_hint(&scope, 0x009b_ff10).unwrap();
    assert!(hint.contains("B::0x009bff00"), "got: {hint}");
    assert!(hint.contains("distance 16"), "got: {hint}");
}

#[test]
fn nearest_stub_hint_honors_pre_filtered_scope() {
    let mods = [
        module("A", &[(0x1, 0x009b_f000), (0x2, 0x009b_f100)]),
        module("B", &[(0x3, 0x009b_ff00)]),
    ];
    let scope_a: Vec<&_> = mods.iter().filter(|m| m.name == "A").collect();
    let hint = nearest_stub_hint(&scope_a, 0x009b_ff10).unwrap();
    assert!(hint.contains("A::0x009bf100"), "got: {hint}");
    assert!(!hint.contains("B::"), "scope leaked: {hint}");
}

#[test]
fn nearest_stub_hint_returns_none_on_empty_scope() {
    let empty: Vec<&cellgov_ppu::prx::ImportedModule> = Vec::new();
    assert!(nearest_stub_hint(&empty, 0x100).is_none());
    let mods = [module("X", &[])];
    let scope: Vec<&_> = mods.iter().collect();
    assert!(nearest_stub_hint(&scope, 0x100).is_none());
}

// -- module_identity ---------------------------------------------------

/// Minimal ELF64-BE header carrying `e_type`, enough for `parse_prx`
/// to reach its e_type check.
fn elf64_be_of_type(e_type: u16) -> Vec<u8> {
    let mut data = vec![0u8; 128];
    data[0..4].copy_from_slice(&cellgov_ps3_abi::format::elf::ELF_MAGIC);
    data[4] = 2; // ELFCLASS64
    data[5] = 2; // ELFDATA2MSB
    data[16..18].copy_from_slice(&e_type.to_be_bytes());
    data
}

#[test]
fn a_title_executable_has_no_module_info_and_that_is_not_a_refusal() {
    let eboot = elf64_be_of_type(cellgov_ps3_abi::format::elf::ET_EXEC);
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
        Err(cellgov_ppu::sprx::PrxParseError::NotPrx(1))
    ));
}
