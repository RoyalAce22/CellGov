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
    v[0..4].copy_from_slice(&cellgov_ps3_abi::format::sce::SCE_MAGIC);
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
fn the_nearest_stub_hint_names_the_target_module_stub_and_distance() {
    let mods = [
        module("A", &[(0x1, 0x009b_f000), (0x2, 0x009b_f100)]),
        module("B", &[(0x3, 0x009b_ff00)]),
    ];
    assert_eq!(
        nearest_stub_hint(&mods, 0x009b_ff10).as_deref(),
        Some(
            "no exact match for 0x009bff10; nearest declared stub is \
             B::0x009bff00 (distance 16 byte(s))"
        )
    );
}

#[test]
fn no_declared_stub_gives_no_hint() {
    let mods = [module("X", &[])];
    assert!(nearest_stub_hint(&mods, 0x100).is_none());
}
