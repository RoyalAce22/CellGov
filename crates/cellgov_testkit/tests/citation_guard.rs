//! Convention guard: every documentation citation is well formed.
//!
//! A citation is `[DOC-KEY p:PAGE]`, optionally followed by
//! `s:SECTION` text, in a comment or a public document. The key names
//! one of the documents below and the page follows that document's
//! own page-identifier grammar, so a reviewer can resolve any tag to
//! its printed page without guessing which book "p:34" means.

use std::fs;
use std::path::{Path, PathBuf};

/// Page-identifier grammar of one cited document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageGrammar {
    /// A plain page number.
    Integer,
    /// A Roman-numeral preface page or an integer body page.
    RomanOrInteger,
    /// `<chapter>-<page>`, `<Appendix letter>-<page>`, `Glossary-<page>`,
    /// `Index-<page>`, or a Roman-numeral preface page.
    ChapterPage,
    /// `I<page>`, `II<page>`, `III<page>` or `App<page>` with one page
    /// count running across all four books, or a Roman-numeral preface
    /// page.
    BookPage,
    /// `<article>:<page>`, the stamp an ACM article-numbered journal
    /// puts on each page.
    ArticlePage,
}

/// The documents a citation may name and how each one numbers its pages.
const DOCUMENTS: &[(&str, PageGrammar)] = &[
    ("PowerISA-3.1", PageGrammar::BookPage),
    ("PPC-Book1", PageGrammar::RomanOrInteger),
    ("PPC-Book2", PageGrammar::RomanOrInteger),
    ("PPC-Book3", PageGrammar::RomanOrInteger),
    ("CBE-Handbook", PageGrammar::Integer),
    ("CBEA", PageGrammar::Integer),
    ("SPU-ISA", PageGrammar::Integer),
    ("AltiVec-PEM", PageGrammar::ChapterPage),
    ("AltiVec-PIM", PageGrammar::ChapterPage),
    ("Bala2000", PageGrammar::Integer),
    ("Brunthaler2010", PageGrammar::Integer),
    ("ErtlGregg2003", PageGrammar::Integer),
    ("Abdulla2017", PageGrammar::ArticlePage),
    ("FlanaganGodefroid2005", PageGrammar::Integer),
    ("Aronis2018", PageGrammar::Integer),
    ("Kokologiannakis2022", PageGrammar::ArticlePage),
    ("Abdulla2024", PageGrammar::Integer),
    ("Nguyen2018", PageGrammar::Integer),
    ("Chalupa2018", PageGrammar::ArticlePage),
    ("Albert2019", PageGrammar::Integer),
    ("Abdulla2019", PageGrammar::ArticlePage),
    ("Mazurkiewicz1977", PageGrammar::Integer),
    ("Godefroid1996", PageGrammar::Integer),
    ("ValmariHansen2016", PageGrammar::Integer),
    ("Lamport1978", PageGrammar::Integer),
    ("FlanaganFreund2009", PageGrammar::Integer),
    ("Rodriguez2015", PageGrammar::Integer),
    ("Kokologiannakis2024", PageGrammar::Integer),
    ("McKeeman1998", PageGrammar::Integer),
    ("Martignoni2009", PageGrammar::Integer),
    ("Yang2011", PageGrammar::Integer),
    ("Regehr2012", PageGrammar::Integer),
    ("Chen2013", PageGrammar::Integer),
    ("Le2014", PageGrammar::Integer),
    ("Veggalam2016", PageGrammar::Integer),
    ("Petsios2017", PageGrammar::Integer),
    ("Klees2018", PageGrammar::Integer),
    ("Armstrong2019", PageGrammar::ArticlePage),
    ("Padhye2019", PageGrammar::Integer),
    ("Manes2021", PageGrammar::Integer),
    ("Jiang2022", PageGrammar::Integer),
    ("Watt2023", PageGrammar::ArticlePage),
    ("Krook2023", PageGrammar::Integer),
    ("Wang2024", PageGrammar::ArticlePage),
    ("Feng2026", PageGrammar::Integer),
];

/// Floor on the population the guard validates. The tree carries
/// several hundred citations; a collapse below this means the scanner
/// stopped recognising the tag.
const MIN_VALID_CITATIONS: usize = 600;

fn is_integer(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_roman(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| matches!(b, b'i' | b'v' | b'x' | b'l' | b'c' | b'd' | b'm'))
}

fn page_matches(grammar: PageGrammar, page: &str) -> bool {
    match grammar {
        PageGrammar::Integer => is_integer(page),
        PageGrammar::RomanOrInteger => is_integer(page) || is_roman(page),
        PageGrammar::ChapterPage => {
            if is_roman(page) {
                return true;
            }
            let Some((chapter, number)) = page.split_once('-') else {
                return false;
            };
            let chapter_ok = is_integer(chapter)
                || (chapter.len() == 1 && chapter.bytes().all(|b| b.is_ascii_uppercase()))
                || chapter == "Glossary"
                || chapter == "Index";
            chapter_ok && is_integer(number)
        }
        PageGrammar::BookPage => {
            if is_roman(page) {
                return true;
            }
            ["III", "II", "I", "App"]
                .iter()
                .find_map(|book| page.strip_prefix(book))
                .is_some_and(is_integer)
        }
        PageGrammar::ArticlePage => page
            .split_once(':')
            .is_some_and(|(article, number)| is_integer(article) && is_integer(number)),
    }
}

/// What the scanner found wrong with one tag.
#[derive(Debug, PartialEq, Eq)]
enum Fault {
    UnknownKey(String),
    NoPage(String),
    BadPage { key: String, page: String },
    DeprecatedForm(String),
}

/// A tag's fault, or `None` for a well-formed one.
fn check_tag(body: &str) -> Option<Fault> {
    let mut words = body.split_whitespace();
    let key = words.next().unwrap_or_default();
    let known = DOCUMENTS.iter().find(|(name, _)| *name == key);
    let Some((_, grammar)) = known else {
        return Some(Fault::UnknownKey(key.to_string()));
    };
    if body.contains("--pdf") {
        return Some(Fault::DeprecatedForm(body.to_string()));
    }
    let Some(page) = words.next().and_then(|w| w.strip_prefix("p:")) else {
        return Some(Fault::NoPage(body.to_string()));
    };
    let page = page.trim_end_matches([',', ';', '.']);
    if page_matches(*grammar, page) {
        None
    } else {
        Some(Fault::BadPage {
            key: key.to_string(),
            page: page.to_string(),
        })
    }
}

/// Whether a bracket body looks like a citation attempt: its first word
/// names a document, or a later word is a `p:` page.
fn looks_like_citation(body: &str) -> bool {
    let mut words = body.split_whitespace();
    let first = words.next().unwrap_or_default();
    DOCUMENTS.iter().any(|(name, _)| *name == first)
        || body.split_whitespace().skip(1).any(|w| w.starts_with("p:"))
}

/// Prose citation forms the schema retired. Each carries a book name
/// where the tag form carries a key.
fn deprecated_prose(line: &str) -> Option<String> {
    if line.contains('\u{a7}') {
        return Some("section symbol".to_string());
    }
    for book in ["Book I ", "Book II ", "Book III "] {
        for rest in line.split(book).skip(1) {
            let next = rest.trim_start();
            let names_a_section = next.starts_with(|c: char| c.is_ascii_digit())
                || next.get(..3).is_some_and(|w| w.eq_ignore_ascii_case("sec"));
            if names_a_section {
                return Some(format!(
                    "{book}{}",
                    next.split_whitespace().next().unwrap_or_default()
                ));
            }
        }
    }
    None
}

/// Scan one line: `(valid tags, faults)`.
///
/// A tag's key and page sit on the line that opens it, so the scanner
/// checks a tag whose `s:` text wraps onto the next line as far as
/// this line goes.
fn scan_line(line: &str) -> (usize, Vec<Fault>) {
    let mut valid = 0;
    let mut faults = Vec::new();
    if let Some(form) = deprecated_prose(line) {
        faults.push(Fault::DeprecatedForm(form));
    }
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let close = after.find(']');
        let body = close.map_or(after, |c| &after[..c]);
        if looks_like_citation(body) {
            match check_tag(body) {
                None => valid += 1,
                Some(fault) => faults.push(fault),
            }
        }
        let Some(close) = close else { break };
        rest = &after[close + 1..];
    }
    (valid, faults)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

fn files_under(dir: &Path, want: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
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
            files_under(&path, want, out);
        } else if want(&path) {
            out.push(path);
        }
    }
}

/// The guards whose own subject is a rule over source text. Each one
/// spells out the shapes it rejects, so the rule it defines does not
/// apply to it.
fn defines_a_rule(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with("_guard.rs"))
}

/// Shipped Rust plus the public documents: `README.md` and `docs/`
/// without its `dev/` subtree, which git does not track.
fn scanned_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let rust = |p: &Path| p.extension().is_some_and(|e| e == "rs") && !defines_a_rule(p);
    for group in ["crates", "apps", "bridges"] {
        files_under(&root.join(group), &rust, &mut found);
    }
    let markdown = |p: &Path| p.extension().is_some_and(|e| e == "md");
    let docs = root.join("docs");
    let entries =
        fs::read_dir(&docs).unwrap_or_else(|e| panic!("cannot read {}: {e}", docs.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", docs.display()))
            .path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "dev") {
                continue;
            }
            files_under(&path, &markdown, &mut found);
        } else if markdown(&path) {
            found.push(path);
        }
    }
    found.push(root.join("README.md"));
    found
}

#[test]
fn every_page_grammar_accepts_its_own_forms_and_rejects_the_others() {
    for (page, ok) in [
        ("34", true),
        ("i", false),
        ("I26", false),
        ("3-4", false),
        ("", false),
    ] {
        assert_eq!(
            page_matches(PageGrammar::Integer, page),
            ok,
            "integer {page:?}"
        );
    }
    for (page, ok) in [("34", true), ("xiv", true), ("I26", false), ("3-4", false)] {
        assert_eq!(
            page_matches(PageGrammar::RomanOrInteger, page),
            ok,
            "roman-or-integer {page:?}"
        );
    }
    for (page, ok) in [
        ("3-12", true),
        ("A-1", true),
        ("Glossary-3", true),
        ("Index-2", true),
        ("xv", true),
        ("34", false),
        ("I26", false),
        ("AB-1", false),
        ("3-", false),
    ] {
        assert_eq!(
            page_matches(PageGrammar::ChapterPage, page),
            ok,
            "chapter-page {page:?}"
        );
    }
    for (page, ok) in [
        ("I26", true),
        ("II1027", true),
        ("III1200", true),
        ("App1500", true),
        ("xxiv", true),
        ("26", false),
        ("IV1", false),
        ("I", false),
        ("3-4", false),
    ] {
        assert_eq!(
            page_matches(PageGrammar::BookPage, page),
            ok,
            "book-page {page:?}"
        );
    }
    for (page, ok) in [
        ("42:11", true),
        ("42", false),
        ("42:", false),
        (":11", false),
        ("42:11:2", false),
    ] {
        assert_eq!(
            page_matches(PageGrammar::ArticlePage, page),
            ok,
            "article-page {page:?}"
        );
    }
}

#[test]
fn an_article_numbered_document_needs_both_halves_of_its_stamp() {
    let (valid, faults) = scan_line("/// the race rule [Abdulla2017 p:42:11 s:3.3]");
    assert_eq!((valid, faults.len()), (1, 0));

    // A bare page number names no article, so it resolves to nothing
    // in a volume that restarts the count per article.
    let (_, faults) = scan_line("/// [Abdulla2017 p:11] a page with no article");
    assert_eq!(
        faults,
        vec![Fault::BadPage {
            key: "Abdulla2017".to_string(),
            page: "11".to_string()
        }]
    );
}

#[test]
fn the_scanner_separates_well_formed_tags_from_each_fault() {
    let (valid, faults) = scan_line("// [PPC-Book1 p:34 s:2.4.2] the rule");
    assert_eq!((valid, faults.len()), (1, 0));

    let (valid, faults) =
        scan_line("// [PowerISA-3.1 p:I26] and [CBE-Handbook p:479 s:18.6.4] twice");
    assert_eq!((valid, faults.len()), (2, 0));

    let (valid, faults) = scan_line("// [AltiVec-PIM p:A-1] appendix");
    assert_eq!((valid, faults.len()), (1, 0));

    let (_, faults) = scan_line("// [PPC-Book4 p:34] no such book");
    assert_eq!(faults, vec![Fault::UnknownKey("PPC-Book4".to_string())]);

    let (_, faults) = scan_line("// [PPC-Book1 s:2.4 p:34] section before page");
    assert_eq!(
        faults,
        vec![Fault::NoPage("PPC-Book1 s:2.4 p:34".to_string())]
    );

    let (_, faults) = scan_line("// [PowerISA-3.1 p:26] bare page on a book-page document");
    assert_eq!(
        faults,
        vec![Fault::BadPage {
            key: "PowerISA-3.1".to_string(),
            page: "26".to_string()
        }]
    );

    let (_, faults) = scan_line("// [PPC-Book1 p:34 --pdf] a tool flag inside the tag");
    assert_eq!(faults.len(), 1);
    assert!(matches!(faults[0], Fault::DeprecatedForm(_)));

    let (_, faults) = scan_line("// Book I 2.4.2 says so");
    assert_eq!(
        faults,
        vec![Fault::DeprecatedForm("Book I 2.4.2".to_string())]
    );

    let (_, faults) = scan_line("// Book III Sec. 3.3.1");
    assert_eq!(
        faults,
        vec![Fault::DeprecatedForm("Book III Sec.".to_string())]
    );

    let (_, faults) = scan_line("// \u{a7}2.4.3 of the manual");
    assert_eq!(
        faults,
        vec![Fault::DeprecatedForm("section symbol".to_string())]
    );

    let (valid, faults) =
        scan_line("// [`Lv2Host`] and [link](x.md) and args[0] are not citations");
    assert_eq!((valid, faults.len()), (0, 0));

    let (valid, faults) = scan_line("// Power ISA Book I is the user set");
    assert_eq!((valid, faults.len()), (0, 0));
}

#[test]
fn a_tag_whose_section_text_wraps_past_the_line_is_still_checked() {
    let (valid, faults) = scan_line("//! walk per [CBE-Handbook p:398 s:14.3.1.3 Figure 14-3");
    assert_eq!((valid, faults.len()), (1, 0));

    let (valid, faults) =
        scan_line("//! PPE 64-Bit Standard Stack Frame] and [AltiVec-PIM p:3-4 s:3.3]:");
    assert_eq!((valid, faults.len()), (1, 0));

    let (_, faults) = scan_line("/// per [PowerISA-3.1 p:26 s:Branch");
    assert_eq!(
        faults,
        vec![Fault::BadPage {
            key: "PowerISA-3.1".to_string(),
            page: "26".to_string()
        }]
    );

    let (_, faults) = scan_line("/// per [PPC-Book1");
    assert_eq!(faults, vec![Fault::NoPage("PPC-Book1".to_string())]);

    let (valid, faults) = scan_line("    let x = arr[i");
    assert_eq!((valid, faults.len()), (0, 0));
}

#[test]
fn the_retired_prose_forms_match_in_either_case_and_past_the_first_mention() {
    let (_, faults) = scan_line("// PPC ISA v2.02 Book I sec. 1.12.2");
    assert_eq!(
        faults,
        vec![Fault::DeprecatedForm("Book I sec.".to_string())]
    );

    let (_, faults) = scan_line("// Book I is the user set; see Book I 2.4.2");
    assert_eq!(
        faults,
        vec![Fault::DeprecatedForm("Book I 2.4.2".to_string())]
    );
}

#[test]
fn every_citation_names_a_known_document_and_a_page_in_its_grammar() {
    let root = workspace_root();
    let files = scanned_files(&root);
    let mut valid = 0usize;
    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for (n, line) in source.lines().enumerate() {
            let (ok, faults) = scan_line(line);
            valid += ok;
            for fault in faults {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                violations.push(format!("  {}:{}  {fault:?}\n", shown.display(), n + 1));
            }
        }
    }
    violations.sort();
    assert!(
        valid >= MIN_VALID_CITATIONS,
        "gate went vacuous: only {valid} well-formed citation(s) recognised across {} files, \
         expected at least {MIN_VALID_CITATIONS}",
        files.len()
    );
    assert!(
        violations.is_empty(),
        "{} citation(s) do not follow `[DOC-KEY p:PAGE s:SECTION]` with a known key and that \
         document's page grammar (integer; Roman or integer for the PPC books; chapter-page \
         for the AltiVec manuals; I/II/III/App-prefixed for the Power ISA):\n{}",
        violations.len(),
        violations.concat()
    );
}
