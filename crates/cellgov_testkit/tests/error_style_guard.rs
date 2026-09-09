//! Convention guards on error types and error-carrying signatures.
//!
//! Every error type derives `thiserror::Error`; its messages start in
//! lower case unless the first word is an acronym or proper noun,
//! render addresses with an explicit `0x` prefix and a fixed width
//! rather than `{:#x}`, and render a chained source through `Display`,
//! never `Debug`. A public signature returns a typed error, not a
//! string, a unit, an integer, or a boxed trait object. The comparison
//! and exploration crates carry no floating-point field, so their
//! observation and result types serialize without a lossy path.

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::TokenTree;

/// First words a message may start with in upper case.
const PROPER_NOUNS: &[&str] = &["Cell"];

/// Crates whose sources carry no `f32` / `f64`.
const FLOAT_FREE_CRATES: &[&str] = &["cellgov_compare", "cellgov_explore"];

/// Floors on the populations the guard polices. A collapse below one
/// means the parser stopped recognising the shape it checks.
const MIN_MESSAGES: usize = 300;
const MIN_PUBLIC_RESULT_SIGNATURES: usize = 150;
const MIN_FLOAT_FREE_FILES: usize = 20;

#[derive(Debug, PartialEq, Eq)]
enum Fault {
    /// A `pub` signature returns `Result<_, E>` with an untyped `E`.
    UntypedError { item: String, error: String },
    /// A hand-written `Display` or `Error` impl on an error type.
    ManualImpl { item: String, trait_name: String },
    /// A message whose first word is capitalised prose.
    UpperCaseMessage { item: String, message: String },
    /// A message rendering a number with `{:#x}`.
    AlternateHex { item: String, message: String },
    /// A message rendering the source field with `Debug`.
    DebugSource { item: String, message: String },
    /// A float in a float-free crate.
    Float { item: String },
}

fn is_error_type_name(ident: &str) -> bool {
    ident.ends_with("Error") || ident.ends_with("Err")
}

fn is_public(vis: &syn::Visibility) -> bool {
    matches!(
        vis,
        syn::Visibility::Public(_) | syn::Visibility::Restricted(_)
    )
}

fn last_ident(path: &syn::Path) -> String {
    path.segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

fn path_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// The untyped shape of an error type, if it is one.
fn untyped_error(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Tuple(t) if t.elems.is_empty() => Some("()".to_string()),
        syn::Type::Reference(r) => match &*r.elem {
            syn::Type::Path(p) if last_ident(&p.path) == "str" => Some("&str".to_string()),
            _ => None,
        },
        syn::Type::Path(p) => {
            let name = path_string(&p.path);
            let last = last_ident(&p.path);
            let integer = [
                "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
                "isize",
            ];
            if last == "String" || integer.contains(&last.as_str()) {
                return Some(last);
            }
            if name == "anyhow::Error" || name == "eyre::Report" {
                return Some(name);
            }
            if last == "Box" {
                if let syn::PathArguments::AngleBracketed(args) =
                    &p.path.segments.last().map(|s| &s.arguments)?
                {
                    if let Some(syn::GenericArgument::Type(syn::Type::TraitObject(obj))) =
                        args.args.first()
                    {
                        let names_error = obj.bounds.iter().any(|b| match b {
                            syn::TypeParamBound::Trait(t) => last_ident(&t.path) == "Error",
                            _ => false,
                        });
                        if names_error {
                            return Some("Box<dyn Error>".to_string());
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// `Some(error type)` when the return type is `Result<_, E>`.
fn result_error_type(output: &syn::ReturnType) -> Option<&syn::Type> {
    let syn::ReturnType::Type(_, ty) = output else {
        return None;
    };
    let syn::Type::Path(p) = &**ty else {
        return None;
    };
    let last = p.path.segments.last()?;
    if last.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.iter().nth(1)? {
        syn::GenericArgument::Type(e) => Some(e),
        _ => None,
    }
}

/// The message literal of an `#[error("...")]` attribute.
fn error_message(attr: &syn::Attribute) -> Option<String> {
    if !attr.path().is_ident("error") {
        return None;
    }
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let first = list.tokens.clone().into_iter().next()?;
    let TokenTree::Literal(lit) = first else {
        return None;
    };
    match syn::Lit::new(lit) {
        syn::Lit::Str(s) => Some(s.value()),
        _ => None,
    }
}

/// Names of the fields a message may not render with `Debug`: those
/// tagged `#[source]` or `#[from]`, and one literally named `source`.
/// Unnamed fields are reported by index.
fn source_fields(fields: &syn::Fields) -> Vec<String> {
    let tagged = |attrs: &[syn::Attribute]| {
        attrs
            .iter()
            .any(|a| a.path().is_ident("source") || a.path().is_ident("from"))
    };
    match fields {
        syn::Fields::Named(named) => named
            .named
            .iter()
            .filter(|f| tagged(&f.attrs) || f.ident.as_ref().is_some_and(|i| i == "source"))
            .filter_map(|f| f.ident.as_ref().map(ToString::to_string))
            .collect(),
        syn::Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .filter(|(_, f)| tagged(&f.attrs))
            .map(|(i, _)| i.to_string())
            .collect(),
        syn::Fields::Unit => Vec::new(),
    }
}

/// Whether the message's first word is capitalised prose rather than
/// an acronym, a proper noun, or a Rust identifier.
fn starts_upper_case_prose(message: &str) -> bool {
    let first = message.split_whitespace().next().unwrap_or_default();
    let Some(c) = first.chars().next() else {
        return false;
    };
    if !c.is_ascii_uppercase() {
        return false;
    }
    let bare = first.trim_end_matches([':', ',', '.']);
    let acronym = !bare.chars().any(|c| c.is_ascii_lowercase());
    let identifier = bare.contains("::")
        || (bare.chars().skip(1).any(|c| c.is_ascii_uppercase()) && !bare.contains(' '));
    !(acronym || identifier || PROPER_NOUNS.contains(&bare))
}

fn uses_alternate_hex(message: &str) -> bool {
    message.split('{').skip(1).any(|spec| {
        let spec = spec.split('}').next().unwrap_or_default();
        let Some((_, fmt)) = spec.split_once(':') else {
            return false;
        };
        fmt.starts_with('#') && fmt.ends_with('x')
    })
}

fn debug_formats_source(message: &str, sources: &[String]) -> bool {
    message.split('{').skip(1).any(|spec| {
        let spec = spec.split('}').next().unwrap_or_default();
        let Some((name, fmt)) = spec.split_once(':') else {
            return false;
        };
        fmt.contains('?') && sources.iter().any(|s| s == name)
    })
}

fn check_messages(
    item: &str,
    attrs: &[syn::Attribute],
    fields: &syn::Fields,
    faults: &mut Vec<Fault>,
) -> usize {
    let mut checked = 0;
    let sources = source_fields(fields);
    for attr in attrs {
        let Some(message) = error_message(attr) else {
            continue;
        };
        checked += 1;
        if starts_upper_case_prose(&message) {
            faults.push(Fault::UpperCaseMessage {
                item: item.to_string(),
                message: message.clone(),
            });
        }
        if uses_alternate_hex(&message) {
            faults.push(Fault::AlternateHex {
                item: item.to_string(),
                message: message.clone(),
            });
        }
        if debug_formats_source(&message, &sources) {
            faults.push(Fault::DebugSource {
                item: item.to_string(),
                message,
            });
        }
    }
    checked
}

/// Tallies from one pass over a file's items.
#[derive(Default, Debug, PartialEq, Eq)]
struct Tally {
    messages: usize,
    public_result_signatures: usize,
}

fn check_signature(
    item: &str,
    vis: &syn::Visibility,
    sig: &syn::Signature,
    tally: &mut Tally,
    faults: &mut Vec<Fault>,
) {
    if !is_public(vis) {
        return;
    }
    let Some(error) = result_error_type(&sig.output) else {
        return;
    };
    tally.public_result_signatures += 1;
    if let Some(shape) = untyped_error(error) {
        if sig.ident == "main" && shape == "Box<dyn Error>" {
            return;
        }
        faults.push(Fault::UntypedError {
            item: item.to_string(),
            error: shape,
        });
    }
}

fn check_items(items: &[syn::Item], tally: &mut Tally, faults: &mut Vec<Fault>) {
    for item in items {
        match item {
            syn::Item::Fn(f) => {
                check_signature(&f.sig.ident.to_string(), &f.vis, &f.sig, tally, faults);
            }
            syn::Item::Impl(imp) => {
                let self_name = match &*imp.self_ty {
                    syn::Type::Path(p) => last_ident(&p.path),
                    _ => String::new(),
                };
                if let Some((_, trait_path, _)) = &imp.trait_ {
                    let trait_name = last_ident(trait_path);
                    if is_error_type_name(&self_name)
                        && (trait_name == "Display" || trait_name == "Error")
                    {
                        faults.push(Fault::ManualImpl {
                            item: self_name.clone(),
                            trait_name,
                        });
                    }
                }
                for member in &imp.items {
                    if let syn::ImplItem::Fn(f) = member {
                        let name = format!("{self_name}::{}", f.sig.ident);
                        check_signature(&name, &f.vis, &f.sig, tally, faults);
                    }
                }
            }
            syn::Item::Enum(e) if is_error_type_name(&e.ident.to_string()) => {
                for v in &e.variants {
                    let name = format!("{}::{}", e.ident, v.ident);
                    tally.messages += check_messages(&name, &v.attrs, &v.fields, faults);
                }
            }
            syn::Item::Struct(s) if is_error_type_name(&s.ident.to_string()) => {
                tally.messages += check_messages(&s.ident.to_string(), &s.attrs, &s.fields, faults);
            }
            syn::Item::Mod(m) => {
                if let Some((_, nested)) = &m.content {
                    check_items(nested, tally, faults);
                }
            }
            _ => {}
        }
    }
}

/// Whether any token of the source is the identifier `f32` or `f64`.
/// Tokens exclude comments and the insides of string literals.
fn mentions_float(source: &str) -> bool {
    fn walk(stream: proc_macro2::TokenStream) -> bool {
        stream.into_iter().any(|tt| match tt {
            TokenTree::Ident(i) => i == "f32" || i == "f64",
            TokenTree::Group(g) => walk(g.stream()),
            _ => false,
        })
    }
    let stream: proc_macro2::TokenStream = source.parse().expect("source tokenizes");
    walk(stream)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

fn rs_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", dir.display()));
        let path = entry.path();
        let kind = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()));
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "target" || n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            rs_files_under(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Whether `path` is test code: under a `tests/` directory or named
/// `*_tests.rs`. `path` must be repo-relative.
fn is_test_file(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_string_lossy() == "tests")
        || path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with("_tests.rs"))
}

/// Shipped, non-test Rust under the three crate groups.
fn shipped_sources(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        rs_files_under(&root.join(group), &mut found);
    }
    found.retain(|f| !is_test_file(f.strip_prefix(root).unwrap_or(f)));
    found.sort();
    found
}

fn parse(path: &Path) -> syn::File {
    let source =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    syn::parse_file(&source).unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()))
}

fn run(source: &str) -> (Tally, Vec<Fault>) {
    let file = syn::parse_file(source).expect("test source parses");
    let mut tally = Tally::default();
    let mut faults = Vec::new();
    check_items(&file.items, &mut tally, &mut faults);
    (tally, faults)
}

#[test]
fn the_signature_check_separates_typed_errors_from_the_untyped_shapes() {
    let (tally, faults) = run("
        pub fn a() -> Result<u32, ParseError> { todo!() }
        pub(crate) fn b() -> Result<(u64, u8), CliArgError> { todo!() }
        fn private() -> Result<(), String> { todo!() }
        pub fn main() -> Result<(), Box<dyn std::error::Error>> { todo!() }
        pub struct S;
        impl S { pub fn m(&self) -> Result<(), io::Error> { todo!() } }
    ");
    assert_eq!(tally.public_result_signatures, 4);
    assert!(faults.is_empty(), "{faults:?}");

    let (_, faults) = run("
        pub fn s() -> Result<(), String> { todo!() }
        pub fn t() -> Result<u8, &'static str> { todo!() }
        pub fn u() -> Result<u8, ()> { todo!() }
        pub fn v() -> Result<u8, u32> { todo!() }
        pub fn w() -> Result<u8, Box<dyn std::error::Error>> { todo!() }
        pub fn x() -> Result<u8, anyhow::Error> { todo!() }
    ");
    let shapes: Vec<&str> = faults
        .iter()
        .map(|f| match f {
            Fault::UntypedError { error, .. } => error.as_str(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        shapes,
        [
            "String",
            "&str",
            "()",
            "u32",
            "Box<dyn Error>",
            "anyhow::Error"
        ]
    );
}

#[test]
fn the_impl_check_flags_hand_written_display_and_error_on_error_types_only() {
    let (_, faults) = run("
        impl std::fmt::Display for ParseError { }
        impl std::error::Error for ParseError { }
        impl core::fmt::Display for GuestAddr { }
        impl Clone for ParseError { }
    ");
    assert_eq!(
        faults,
        vec![
            Fault::ManualImpl {
                item: "ParseError".into(),
                trait_name: "Display".into()
            },
            Fault::ManualImpl {
                item: "ParseError".into(),
                trait_name: "Error".into()
            },
        ]
    );
}

#[test]
fn the_message_check_accepts_acronyms_and_identifiers_and_rejects_capitalised_prose() {
    let (tally, faults) = run(r#"
        #[derive(thiserror::Error)]
        pub enum LoadError {
            #[error("PRX header short")] A,
            #[error("I/O failed: {source}")] B { #[source] source: io::Error },
            #[error("PARAM.SFO: {0}")] C(String),
            #[error("SCE: bad magic")] D,
            #[error("Cell BE limit")] E,
            #[error("RsxWriteCheckpoint reached")] F,
            #[error("Convergence::No")] G,
            #[error("{0}")] H(u32),
            #[error("segment 0x{addr:08x} unmapped")] I { addr: u32 },
            #[error(transparent)] J(#[from] io::Error),
        }
        #[derive(thiserror::Error)]
        #[error("guest fault at 0x{pc:08x}")]
        pub struct FaultError { pc: u64 }
        #[derive(thiserror::Error)]
        pub enum Outcome { #[error("Completed")] Done }
    "#);
    assert_eq!(
        tally.messages, 10,
        "transparent carries no message; Outcome is no error type"
    );
    assert!(faults.is_empty(), "{faults:?}");

    let (_, faults) = run(r#"
        #[derive(thiserror::Error)]
        pub enum BadError {
            #[error("Parse failed")] A,
            #[error("segment {addr:#x} unmapped")] B { addr: u32 },
            #[error("segment {:#010x}")] C(u32),
            #[error("read failed: {source:?}")] D { #[source] source: io::Error },
            #[error("read failed: {0:?}")] E(#[from] io::Error),
            #[error("read failed: {source:?}")] F { source: io::Error },
            #[error("path {path:?} missing")] G { path: PathBuf },
        }
    "#);
    let kinds: Vec<&str> = faults
        .iter()
        .map(|f| match f {
            Fault::UpperCaseMessage { .. } => "upper",
            Fault::AlternateHex { .. } => "hex",
            Fault::DebugSource { .. } => "debug",
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(kinds, ["upper", "hex", "hex", "debug", "debug", "debug"]);
}

#[test]
fn the_float_check_reads_tokens_not_comments_or_strings() {
    assert!(!mentions_float(
        "// f64 in a comment\nconst S: &str = \"f32\";\nfn f() {}"
    ));
    assert!(mentions_float("pub struct O { pub ratio: f64 }"));
    assert!(mentions_float("fn f() { let x: Vec<f32> = vec![]; }"));
}

#[test]
fn shipped_error_types_and_signatures_follow_the_style() {
    let root = workspace_root();
    let files = shipped_sources(&root);
    let mut tally = Tally::default();
    let mut report = Vec::new();
    for file in &files {
        let mut faults = Vec::new();
        check_items(&parse(file).items, &mut tally, &mut faults);
        let shown = file.strip_prefix(&root).unwrap_or(file);
        for fault in faults {
            report.push(format!("  {}  {fault:?}\n", shown.display()));
        }
    }
    assert!(
        tally.messages >= MIN_MESSAGES && tally.public_result_signatures >= MIN_PUBLIC_RESULT_SIGNATURES,
        "gate went vacuous: {} message(s) and {} public Result signature(s) recognised across {} files",
        tally.messages,
        tally.public_result_signatures,
        files.len()
    );
    report.sort();
    assert!(
        report.is_empty(),
        "{} error-style deviation(s): messages start in lower case unless the first word is an \
         acronym, an identifier or a proper noun; hex renders as 0x{{x:08x}}; a source renders \
         through Display; a public signature returns a typed error:\n{}",
        report.len(),
        report.concat()
    );
}

#[test]
fn the_comparison_and_exploration_crates_carry_no_float() {
    let root = workspace_root();
    let mut files = Vec::new();
    for krate in FLOAT_FREE_CRATES {
        rs_files_under(&root.join("crates").join(krate).join("src"), &mut files);
    }
    files.retain(|f| !is_test_file(f.strip_prefix(&root).unwrap_or(f)));
    assert!(
        files.len() >= MIN_FLOAT_FREE_FILES,
        "gate went vacuous: only {} source file(s) under the float-free crates",
        files.len()
    );
    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        if mentions_float(&source) {
            let shown = file.strip_prefix(&root).unwrap_or(file);
            violations.push(Fault::Float {
                item: shown.display().to_string(),
            });
        }
    }
    assert!(
        violations.is_empty(),
        "a float reached a crate whose observation and result types are declared float-free; \
         the JSON round trip through those types would stop being exact:\n{violations:#?}"
    );
}
