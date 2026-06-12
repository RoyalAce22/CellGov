//! dump-prx-imports argument parsing and name-column formatting.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    let mut v = vec!["cellgov_cli".to_string(), "dump-prx-imports".to_string()];
    v.extend(args.iter().map(|s| s.to_string()));
    v
}

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

// -- try_parse_args ----------------------------------------------------

#[test]
fn try_parse_args_accepts_path_and_lowercase_hex_at() {
    let a = argv(&["/tmp/libfs.prx", "--at", "0x9bff10"]);
    let p = try_parse_args(&a).expect("happy path");
    assert_eq!(p.path.to_string_lossy(), "/tmp/libfs.prx");
    assert_eq!(p.filter_addr, Some(0x009b_ff10));
    assert!(p.filter_module.is_none());
}

#[test]
fn try_parse_args_accepts_uppercase_0x_prefix() {
    let a = argv(&["/tmp/libfs.prx", "--at", "0X9BFF10"]);
    let p = try_parse_args(&a).unwrap();
    assert_eq!(p.filter_addr, Some(0x009b_ff10));
}

#[test]
fn try_parse_args_accepts_bare_hex_no_prefix() {
    let a = argv(&["/tmp/libfs.prx", "--at", "9bff10"]);
    let p = try_parse_args(&a).unwrap();
    assert_eq!(p.filter_addr, Some(0x009b_ff10));
}

#[test]
fn try_parse_args_rejects_non_hex_at_value() {
    let a = argv(&["/tmp/libfs.prx", "--at", "not-hex"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("--at"));
}

#[test]
fn try_parse_args_accepts_module_filter() {
    let a = argv(&["libsysutil.sprx", "--module", "sys_fs"]);
    let p = try_parse_args(&a).unwrap();
    assert_eq!(p.filter_module.as_deref(), Some("sys_fs"));
    assert!(p.filter_addr.is_none());
}

#[test]
fn try_parse_args_accepts_save_elf_path() {
    let a = argv(&["libsysutil.sprx", "--save-elf", "/tmp/out.elf"]);
    let p = try_parse_args(&a).unwrap();
    assert_eq!(
        p.save_elf.as_deref().and_then(|p| p.to_str()),
        Some("/tmp/out.elf")
    );
    assert!(p.filter_addr.is_none());
    assert!(p.filter_module.is_none());
}

#[test]
fn try_parse_args_rejects_save_elf_without_value() {
    let a = argv(&["libsysutil.sprx", "--save-elf"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("--save-elf"), "got: {err}");
}

#[test]
fn try_parse_args_rejects_duplicate_save_elf() {
    let a = argv(&["libsysutil.sprx", "--save-elf", "/a", "--save-elf", "/b"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("--save-elf"), "got: {err}");
    assert!(err.contains("more than once"), "got: {err}");
}

#[test]
fn try_parse_args_save_elf_composes_with_at_and_module() {
    let a = argv(&[
        "libsysutil.sprx",
        "--at",
        "0x9bff10",
        "--module",
        "sys_fs",
        "--save-elf",
        "/tmp/out.elf",
    ]);
    let p = try_parse_args(&a).unwrap();
    assert_eq!(p.filter_addr, Some(0x009b_ff10));
    assert_eq!(p.filter_module.as_deref(), Some("sys_fs"));
    assert_eq!(
        p.save_elf.as_deref().and_then(|p| p.to_str()),
        Some("/tmp/out.elf")
    );
}

#[test]
fn try_parse_args_rejects_duplicate_at() {
    let a = argv(&["/tmp/libfs.prx", "--at", "0x100", "--at", "0x200"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("--at"), "got: {err}");
    assert!(err.contains("more than once"), "got: {err}");
}

#[test]
fn try_parse_args_rejects_duplicate_module() {
    let a = argv(&["/tmp/libfs.prx", "--module", "A", "--module", "B"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("--module"), "got: {err}");
    assert!(err.contains("more than once"), "got: {err}");
}

#[test]
fn try_parse_args_rejects_unknown_flag() {
    let a = argv(&["/tmp/libfs.prx", "--xyzzy"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("--xyzzy"), "got: {err}");
}

#[test]
fn try_parse_args_rejects_two_positional_paths() {
    let a = argv(&["/tmp/a.prx", "/tmp/b.prx"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("positional"), "got: {err}");
}

#[test]
fn try_parse_args_rejects_missing_path() {
    let a = argv(&["--at", "0x100"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("usage"), "got: {err}");
}

#[test]
fn try_parse_args_rejects_at_without_value() {
    let a = argv(&["/tmp/libfs.prx", "--at"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("--at"), "got: {err}");
}

#[test]
fn try_parse_args_rejects_module_without_value() {
    let a = argv(&["/tmp/libfs.prx", "--module"]);
    let err = try_parse_args(&a).unwrap_err();
    assert!(err.contains("--module"), "got: {err}");
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
    let buf = pad_with_elf_magic(cellgov_ps3_abi::elf::ELF_HEADER_SIZE);
    assert_eq!(classify_source(&buf), Ok(SourceKind::Elf));
}

#[test]
fn classify_source_routes_sce_magic_to_sce_wrapped() {
    let buf = pad_with_sce_magic(cellgov_ps3_abi::elf::ELF_HEADER_SIZE);
    assert_eq!(classify_source(&buf), Ok(SourceKind::SceWrapped));
}

#[test]
fn classify_source_rejects_short_file_even_with_valid_magic() {
    let buf = ELF_MAGIC.to_vec();
    assert_eq!(classify_source(&buf), Err(LoadError::TooSmall { len: 4 }));
}

#[test]
fn classify_source_rejects_bad_magic() {
    let mut buf = vec![0u8; cellgov_ps3_abi::elf::ELF_HEADER_SIZE];
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
