//! Convention guard: every documentation citation is well formed.
//!
//! A citation is `[DOC-KEY p:PAGE]`, optionally followed by
//! `s:SECTION` text, in a comment or a public document. The key names
//! one of the documents below and the page follows that document's
//! own page-identifier grammar, so a reviewer can resolve any tag to
//! its printed page without guessing which book "p:34" means.
//!
//! A web page is cited as `[WEB-KEY s:SECTION]`. The key names one of
//! the web sources below, which holds the page's archived snapshot.
//! The snapshot URL lives only in that table: a raw psdevwiki or
//! Wayback link anywhere else in the scanned files is a fault.
//!
//! A fact read from a firmware image is cited as
//! `[OFW-<version> <image> a:<addr>[-<addr>]]`. The version is a row of
//! the archive's PUP table. The image is `lv1`, `lv2_kernel`, or the
//! `dev_flash` path of a module that the version's PUP ships, such as
//! `sys/external/liblv2.sprx`. The address is the one
//! `cellgov dev disasm` prints for that image.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
    ("Lubachevsky1989", PageGrammar::Integer),
    ("Fujimoto1990", PageGrammar::Integer),
    ("Olszewski2009", PageGrammar::Integer),
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
    ("LemireKaser2014", PageGrammar::Integer),
    ("CarterWegman1979", PageGrammar::Integer),
    ("Black1999", PageGrammar::Integer),
    ("WegmanCarter1981", PageGrammar::Integer),
    ("Clarke2003", PageGrammar::Integer),
    ("Lewi2019", PageGrammar::Integer),
    ("Mueller2005", PageGrammar::Integer),
    ("Monniaux2008", PageGrammar::Integer),
    ("CotaCarloni2019", PageGrammar::Integer),
    ("Boldo2015", PageGrammar::Integer),
    ("Aharoni2003", PageGrammar::Integer),
    ("Verdonk2001Basic", PageGrammar::Integer),
    ("Verdonk2001Conversions", PageGrammar::Integer),
    ("Liew2017", PageGrammar::Integer),
    ("Bansal2006", PageGrammar::Integer),
    ("Schkufza2013", PageGrammar::Integer),
    ("Mullen2016", PageGrammar::Integer),
    ("Liu2024", PageGrammar::ArticlePage),
    ("Menendez2016", PageGrammar::Integer),
    ("Mukherjee2024", PageGrammar::ArticlePage),
    ("Schkufza2014", PageGrammar::Integer),
    ("Chen2018", PageGrammar::ArticlePage),
    ("JiaHarman2011", PageGrammar::Integer),
    ("Necula2000", PageGrammar::Integer),
    ("Lopes2021", PageGrammar::Integer),
    ("Martignoni2012", PageGrammar::Integer),
    ("Kistler2006", PageGrammar::Integer),
];

/// The web pages a citation may name, each by its alias and its one
/// archived snapshot. The 14-digit stamp in each URL is the snapshot's
/// date and time.
///
/// The two LV2 pages hold the snapshots closest before the archive took
/// its rows from them on 2026-09-18.
const WEB_SOURCES: &[(&str, &str)] = &[
    (
        "PS3DEVWIKI-HV-RE",
        "https://web.archive.org/web/20260512043631/https://www.psdevwiki.com/ps3/Hypervisor_Reverse_Engineering",
    ),
    (
        "PS3DEVWIKI-HV-SYSCALLS",
        "https://web.archive.org/web/20260914175021/https://www.psdevwiki.com/ps3/HV_Syscalls",
    ),
    (
        "PS3DEVWIKI-HV-SYSCALL-REF",
        "https://web.archive.org/web/20260724151518/https://www.psdevwiki.com/ps3/HV_Syscall_Reference",
    ),
    (
        "PS3DEVWIKI-LV2-SYSCALLS",
        "https://web.archive.org/web/20260818233001/https://www.psdevwiki.com/ps3/LV2_Functions_and_Syscalls",
    ),
    (
        "PS3DEVWIKI-LV2-SYSCALLS-TALK",
        "https://web.archive.org/web/20260304161811/https://www.psdevwiki.com/ps3/Talk:LV2_Functions_and_Syscalls",
    ),
    (
        "PS3DEVWIKI-REPO",
        "https://web.archive.org/web/20251220062231/https://www.psdevwiki.com/ps3/Repository_Nodes",
    ),
];

/// The hosts a raw link may not name outside [`WEB_SOURCES`].
const ALIASED_HOSTS: &[&str] = &["psdevwiki.com", "web.archive.org"];

/// The key prefix of a firmware citation; the version follows it.
const FIRMWARE_KEY: &str = "OFW-";

/// The CoreOS images a firmware citation may name besides a module.
const CORE_OS_IMAGES: &[&str] = &["lv1", "lv2_kernel"];

/// The archive tables a firmware citation is checked against, relative
/// to the workspace root: the PUP of each firmware version, and every
/// module the extraction scanned in each PUP.
const PUP_TABLE: &str = "docs/lv2/tables/pup.tsv";
const MODULE_TABLE: &str = "docs/lv2/tables/caller_unresolved.tsv";

/// Each firmware version the archive holds a PUP of, with the
/// `dev_flash` paths of the modules that its PUPs ship.
fn firmware_modules() -> &'static BTreeMap<String, BTreeSet<String>> {
    static TABLE: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let root = workspace_root();
        let read = |name: &str| {
            fs::read_to_string(root.join(name))
                .unwrap_or_else(|e| panic!("cannot read {name}: {e}"))
        };
        let pups = read(PUP_TABLE);
        let modules = read(MODULE_TABLE);
        load_firmware_modules(&pups, &modules)
    })
}

/// Parses the PUP table and the module table into
/// [`firmware_modules`]'s map.
///
/// # Panics
///
/// Panics when a table header is not the expected one, or when a module
/// row names a PUP that the PUP table does not hold.
fn load_firmware_modules(pups: &str, modules: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut pup_lines = pups.lines();
    assert!(
        pup_lines
            .next()
            .is_some_and(|h| h.starts_with("pup_sha256\tfw\t")),
        "{PUP_TABLE} header changed"
    );
    let mut firmware_of = BTreeMap::new();
    let mut table: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in pup_lines {
        let mut cells = line.split('\t');
        let (Some(sha), Some(fw)) = (cells.next(), cells.next()) else {
            panic!("{PUP_TABLE} row {line:?} has no fw cell");
        };
        firmware_of.insert(sha, fw);
        table.entry(fw.to_string()).or_default();
    }
    let mut module_lines = modules.lines();
    assert!(
        module_lines
            .next()
            .is_some_and(|h| h.starts_with("pup_sha256\tmodule\t")),
        "{MODULE_TABLE} header changed"
    );
    for line in module_lines {
        let mut cells = line.split('\t');
        let (Some(sha), Some(module)) = (cells.next(), cells.next()) else {
            panic!("{MODULE_TABLE} row {line:?} has no module cell");
        };
        let fw = firmware_of
            .get(sha)
            .unwrap_or_else(|| panic!("{MODULE_TABLE} names PUP {sha}, which {PUP_TABLE} lacks"));
        table
            .get_mut(*fw)
            .expect("every PUP's version has an entry")
            .insert(module.to_string());
    }
    table
}

/// A `0x`-prefixed hexadecimal address of at most 16 digits.
fn parse_address(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("0x")?;
    if digits.is_empty() || digits.len() > 16 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

/// A firmware citation's fault, or `None` for a well-formed one.
fn check_firmware_tag(body: &str) -> Option<Fault> {
    let malformed = || Some(Fault::MalformedFirmwareTag(body.to_string()));
    let mut words = body.split_whitespace();
    let Some(version) = words.next().and_then(|w| w.strip_prefix(FIRMWARE_KEY)) else {
        return malformed();
    };
    let (Some(image), Some(field), None) = (words.next(), words.next(), words.next()) else {
        return malformed();
    };
    let Some(modules) = firmware_modules().get(version) else {
        return Some(Fault::UnknownFirmware(version.to_string()));
    };
    if !CORE_OS_IMAGES.contains(&image) && !modules.contains(image) {
        return Some(Fault::UnknownImage {
            firmware: version.to_string(),
            image: image.to_string(),
        });
    }
    let Some(window) = field.strip_prefix("a:") else {
        return malformed();
    };
    let (start, end) = window.split_once('-').unwrap_or((window, window));
    let (Some(start), Some(end)) = (parse_address(start), parse_address(end)) else {
        return Some(Fault::BadAddress(window.to_string()));
    };
    if end < start {
        return Some(Fault::BackwardWindow(window.to_string()));
    }
    None
}

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
#[derive(Clone, Debug, PartialEq, Eq)]
enum Fault {
    UnknownKey(String),
    NoPage(String),
    BadPage { key: String, page: String },
    DeprecatedForm(String),
    NoSection(String),
    RawWebLink(String),
    MalformedFirmwareTag(String),
    UnknownFirmware(String),
    UnknownImage { firmware: String, image: String },
    BadAddress(String),
    BackwardWindow(String),
    PlainFirmwareCitation(String),
}

/// Whether a key word opens a firmware citation, the operator's
/// spaced `OFW v4.93` sketch included.
fn is_firmware_key(word: &str) -> bool {
    word.starts_with(FIRMWARE_KEY) || word == "OFW"
}

/// A tag's fault, or `None` for a well-formed one.
fn check_tag(body: &str) -> Option<Fault> {
    let mut words = body.split_whitespace();
    let key = words.next().unwrap_or_default();
    if is_firmware_key(key) {
        return check_firmware_tag(body);
    }
    if WEB_SOURCES.iter().any(|(name, _)| *name == key) {
        let section = words.next().and_then(|w| w.strip_prefix("s:"));
        return match section {
            Some(section) if !section.is_empty() => None,
            _ => Some(Fault::NoSection(body.to_string())),
        };
    }
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
/// names a document or carries the web-source alias or firmware key
/// prefix, or a later word is a `p:` page.
fn looks_like_citation(body: &str) -> bool {
    let mut words = body.split_whitespace();
    let first = words.next().unwrap_or_default();
    DOCUMENTS.iter().any(|(name, _)| *name == first)
        || WEB_SOURCES.iter().any(|(name, _)| *name == first)
        || first.starts_with("PS3DEVWIKI-")
        || is_firmware_key(first)
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

/// The words that name a firmware image in prose.
const IMAGE_WORDS: &[&str] = &[
    "lv1",
    "lv1's",
    "lv1.self",
    "lv2_kernel",
    "lv2_kernel.self",
    "kernel",
    "kernel's",
    "hypervisor",
    "hypervisor's",
    "image",
];

/// The image words that the "image at address" form looks for. `image`
/// is left out, because a PUP's `image 0x...` version word is no
/// address.
const CODE_IMAGE_WORDS: &[&str] = &[
    "lv1",
    "lv1's",
    "lv2_kernel",
    "kernel",
    "kernel's",
    "hypervisor",
    "hypervisor's",
];

/// `line` with each firmware tag removed, so that an address inside a
/// tag is not a prose address.
fn without_firmware_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let first = after.split_whitespace().next().unwrap_or_default();
        match after.find(']') {
            Some(close) if is_firmware_key(first) => rest = &after[close + 1..],
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A prose word without its surrounding quotes and punctuation.
fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| {
        matches!(
            c,
            '`' | '(' | ')' | '[' | ']' | ',' | '.' | ';' | ':' | '"' | '*'
        )
    })
}

/// A firmware fact cited in prose rather than by a tag: the form that
/// matched, or `None`. The forms are the ones the tag replaced:
/// - `stock <version> <image>`;
/// - `Firmware reading` on a line that names a version;
/// - `<version> <image>` or `<image> <version>` on a line with a prose
///   address;
/// - `<image> <address>` or `<image> at <address>`;
/// - `at <version> <address>`.
///
/// An address is `0x` and at least four hexadecimal digits that are not
/// an offset (`+0x1C0`, `HSPRG0 - 0x6A80`).
fn plain_firmware_citation(line: &str) -> Option<String> {
    let prose = without_firmware_tags(line);
    let words: Vec<&str> = prose.split_whitespace().collect();
    let versions = firmware_modules();
    let is_version = |w: &str| versions.contains_key(bare(w));
    let is_image = |w: &str| IMAGE_WORDS.contains(&bare(w));
    let is_code_image = |w: &str| CODE_IMAGE_WORDS.contains(&bare(w));
    let is_address = |i: usize| {
        let Some(word) = words.get(i) else {
            return false;
        };
        let offset = i > 0 && matches!(words[i - 1], "-" | "+");
        let digits = bare(word).strip_prefix("0x");
        !offset
            && !word.starts_with(['+', '-'])
            && digits.is_some_and(|d| d.len() >= 4 && d.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let has_address = (0..words.len()).any(is_address);
    let has_version = words.iter().any(|w| is_version(w));
    let shown = |from: usize, count: usize| words[from..(from + count).min(words.len())].join(" ");
    for (i, word) in words.iter().enumerate() {
        let next = |k: usize| words.get(i + k).copied().unwrap_or_default();
        if bare(word).eq_ignore_ascii_case("stock") && is_version(next(1)) && is_image(next(2)) {
            return Some(shown(i, 3));
        }
        if *word == "Firmware" && bare(next(1)) == "reading" && has_version {
            return Some(shown(i, 2));
        }
        let paired =
            (is_version(word) && is_image(next(1))) || (is_image(word) && is_version(next(1)));
        if paired && has_address {
            return Some(shown(i, 2));
        }
        if is_code_image(word)
            && (is_address(i + 1) || (bare(next(1)) == "at" && is_address(i + 2)))
        {
            return Some(shown(i, if is_address(i + 1) { 2 } else { 3 }));
        }
        if bare(word) == "at" && is_version(next(1)) && is_address(i + 2) {
            return Some(shown(i, 3));
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
    if let Some(host) = ALIASED_HOSTS.iter().find(|host| line.contains(*host)) {
        faults.push(Fault::RawWebLink((*host).to_string()));
    }
    if let Some(form) = plain_firmware_citation(line) {
        faults.push(Fault::PlainFirmwareCitation(form));
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

/// Shipped Rust plus the public documents: `README.md`, `docs/`
/// without its `dev/` subtree, which git does not track, and each
/// microtest's `README.md`.
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
    let micro = root.join("tests").join("micro");
    let entries =
        fs::read_dir(&micro).unwrap_or_else(|e| panic!("cannot read {}: {e}", micro.display()));
    for entry in entries {
        let readme = entry
            .unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", micro.display()))
            .path()
            .join("README.md");
        if readme.is_file() {
            found.push(readme);
        }
    }
    found.push(root.join("README.md"));
    found
}

/// Whether `line` opens an item a doc comment documents: a function,
/// constant, static, type, trait, impl, module or union.
fn opens_an_item(line: &str) -> bool {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.strip_prefix('(') {
            Some(scoped) => scoped.split_once(')').map_or("", |(_, tail)| tail),
            None => after,
        };
    }
    let mut words = rest.split_whitespace().peekable();
    while let Some(word) = words.next() {
        let qualifier = matches!(word, "const" | "async" | "unsafe" | "default" | "extern")
            || word.starts_with('"');
        let next_qualifies = words.peek().is_some_and(|next| {
            matches!(
                *next,
                "fn" | "const" | "async" | "unsafe" | "extern" | "impl" | "trait"
            ) || next.starts_with('"')
        });
        if qualifier && next_qualifies {
            continue;
        }
        let keyword = word.split(['<', '(', '{']).next().unwrap_or_default();
        return matches!(
            keyword,
            "fn" | "const"
                | "static"
                | "struct"
                | "enum"
                | "trait"
                | "impl"
                | "type"
                | "mod"
                | "union"
        );
    }
    false
}

/// First lines (1-based) of `//` comment blocks that carry a citation
/// and sit directly on an item, past its `///` docs and attributes.
fn line_comment_citations_on_items(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let plain = |line: &str| {
        let s = line.trim_start();
        s.starts_with("//") && !s.starts_with("///") && !s.starts_with("//!")
    };
    let mut found = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !plain(lines[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && plain(lines[i]) {
            i += 1;
        }
        // A safety argument stays a comment even when it cites a page.
        if lines[start].trim_start().starts_with("// SAFETY")
            || !lines[start..i].iter().any(|l| scan_line(l).0 > 0)
        {
            continue;
        }
        fn code(line: &str) -> &str {
            line.split(" //").next().unwrap_or_default().trim_end()
        }
        let mut j = i;
        while j < lines.len() {
            let s = lines[j].trim_start();
            if s.starts_with("///") {
                j += 1;
            } else if s.starts_with("#[") {
                while j < lines.len() && !code(lines[j]).ends_with(']') {
                    j += 1;
                }
                j += 1;
            } else {
                break;
            }
        }
        if lines.get(j).is_some_and(|l| opens_an_item(l)) {
            found.push(start + 1);
        }
    }
    found
}

#[test]
fn a_citation_block_on_an_item_is_found_past_docs_and_attributes() {
    for (source, line) in [
        ("// [SPU-ISA p:197 s:9.2] the rule\nfn f() {}", 1),
        (
            "/// Docs.\n// [SPU-ISA p:197 s:9.2] the rule\n#[inline]\npub(crate) const fn f() {}",
            2,
        ),
        (
            "    // [CBEA p:109 s:9] the rule\n    pub const X: u32 = 1;",
            1,
        ),
        (
            "// [PPC-Book1 p:34] the rule\n// more prose\nimpl<T> Foo for T {}",
            1,
        ),
        (
            "// [PPC-Book1 p:34] the rule\npub unsafe extern \"C\" fn f() {}",
            1,
        ),
        (
            "// prose first\n// [PPC-Book1 p:34] the rule\n#[derive(\n    Debug,\n)]\nstruct S;",
            1,
        ),
        (
            "// [PPC-Book1 p:34] the rule\n#[allow(dead_code)] // why\nfn f() {\n    let x = 1;\n}",
            1,
        ),
    ] {
        assert_eq!(
            line_comment_citations_on_items(source),
            vec![line],
            "{source}"
        );
    }
    for source in [
        "/// [SPU-ISA p:197 s:9.2] the rule\nfn f() {}",
        "    // [CBEA p:109 s:9] a statement\n    let x = 1;",
        "        // [CBEA p:109 s:9] an arm\n        spu::MFC_LSA => 1,",
        "// [SPU-ISA p:197 s:9.2] a gap\n\nfn f() {}",
        "// no citation here\nfn f() {}",
        "// [SPU-ISA p:197 s:9.2] a call\nconstant_folding();",
        "// [SPU-ISA p:197 s:9.2] a crate\nextern crate alloc;",
        "// SAFETY: [PPC-Book1 p:34] the rule holds\nunsafe fn f() {}",
    ] {
        assert!(
            line_comment_citations_on_items(source).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn no_citation_on_an_item_is_a_line_comment() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for file in scanned_files(&root) {
        if file.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for line in line_comment_citations_on_items(&source) {
            let shown = file.strip_prefix(&root).unwrap_or(&file);
            violations.push(format!("  {}:{line}\n", shown.display()));
        }
    }
    assert!(
        violations.is_empty(),
        "{} `//` citation block(s) sit on an item, where rustdoc drops them; write them as `///`:\n{}",
        violations.len(),
        violations.concat()
    );
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
fn a_web_source_tag_names_a_section_and_a_raw_link_is_a_fault() {
    let (valid, faults) = scan_line("/// the node [PS3DEVWIKI-REPO s:3.41_GameOS] gives 1");
    assert_eq!((valid, faults.len()), (1, 0));

    let (_, faults) = scan_line("/// the node [PS3DEVWIKI-REPO] gives 1");
    assert_eq!(
        faults,
        vec![Fault::NoSection("PS3DEVWIKI-REPO".to_string())]
    );

    let (_, faults) = scan_line("/// [PS3DEVWIKI-REPO p:3] a page where a section belongs");
    assert_eq!(
        faults,
        vec![Fault::NoSection("PS3DEVWIKI-REPO p:3".to_string())]
    );

    let (_, faults) = scan_line("/// the node [PS3DEVWIKI-REPOS s:3.41_GameOS] gives 1");
    assert_eq!(
        faults,
        vec![Fault::UnknownKey("PS3DEVWIKI-REPOS".to_string())]
    );

    let (_, faults) = scan_line("/// see https://www.psdevwiki.com/ps3/Repository_Nodes");
    assert_eq!(faults, vec![Fault::RawWebLink("psdevwiki.com".to_string())]);

    let (_, faults) = scan_line("// \"https://web.archive.org/web/20251220062231/x\"");
    assert_eq!(
        faults,
        vec![Fault::RawWebLink("web.archive.org".to_string())]
    );

    let (valid, faults) = scan_line("/// psdevwiki's page, cited by alias");
    assert_eq!((valid, faults.len()), (0, 0));
}

#[test]
fn every_web_source_holds_one_dated_wayback_snapshot_of_a_psdevwiki_page() {
    assert!(!WEB_SOURCES.is_empty());
    for (alias, url) in WEB_SOURCES {
        assert!(alias.starts_with("PS3DEVWIKI-"), "{alias}");
        let stamp = url
            .strip_prefix("https://web.archive.org/web/")
            .and_then(|rest| rest.split_once("/https://www.psdevwiki.com/ps3/"))
            .map(|(stamp, _)| stamp);
        assert!(
            stamp.is_some_and(|s| s.len() == 14 && s.bytes().all(|b| b.is_ascii_digit())),
            "{alias} holds {url}, which is no dated Wayback snapshot of a psdevwiki page"
        );
    }
    let mut aliases: Vec<&str> = WEB_SOURCES.iter().map(|(alias, _)| *alias).collect();
    aliases.sort_unstable();
    aliases.dedup();
    assert_eq!(aliases.len(), WEB_SOURCES.len(), "an alias repeats");
}

#[test]
fn a_firmware_tag_needs_a_known_version_a_shipped_image_and_a_forward_address() {
    let fault = |line: &str| {
        let (valid, faults) = scan_line(line);
        assert!(faults.len() <= 1, "{line}: {faults:?}");
        (valid, faults.into_iter().next())
    };
    // Each rule: one line that passes it and one that fails it.
    let cases: &[(&str, &str, Option<Fault>)] = &[
        ("version", "/// [OFW-1.50 lv1 a:0x2B9CE0]", None),
        (
            "version",
            "/// [OFW-4.94 lv1 a:0x2EA35C]",
            Some(Fault::UnknownFirmware("4.94".to_string())),
        ),
        (
            "core image",
            "/// [OFW-4.93 lv2_kernel a:0x8000000000007D2C]",
            None,
        ),
        (
            "core image",
            "/// [OFW-4.93 kernel a:0x8000000000007D2C]",
            Some(Fault::UnknownImage {
                firmware: "4.93".to_string(),
                image: "kernel".to_string(),
            }),
        ),
        (
            "module",
            "/// [OFW-1.50 vsh/module/game_plugin.sprx a:0x0]",
            None,
        ),
        (
            "module",
            "/// [OFW-4.93 vsh/module/game_plugin.sprx a:0x0]",
            Some(Fault::UnknownImage {
                firmware: "4.93".to_string(),
                image: "vsh/module/game_plugin.sprx".to_string(),
            }),
        ),
        ("address", "/// [OFW-4.93 lv1 a:0x2ea35c]", None),
        (
            "address",
            "/// [OFW-4.93 lv1 a:2EA35C]",
            Some(Fault::BadAddress("2EA35C".to_string())),
        ),
        (
            "address",
            "/// [OFW-4.93 lv1 a:0x2EA3G0]",
            Some(Fault::BadAddress("0x2EA3G0".to_string())),
        ),
        (
            "address",
            "/// [OFW-4.93 lv1 a:0x10000000000000000]",
            Some(Fault::BadAddress("0x10000000000000000".to_string())),
        ),
        ("window", "/// [OFW-4.93 lv1 a:0x2EA35C-0x2EA3A0]", None),
        (
            "window",
            "/// [OFW-4.93 lv1 a:0x2EA3A0-0x2EA35C]",
            Some(Fault::BackwardWindow("0x2EA3A0-0x2EA35C".to_string())),
        ),
        (
            "window",
            "/// [OFW-4.93 lv1 a:0x2EA35C-]",
            Some(Fault::BadAddress("0x2EA35C-".to_string())),
        ),
        (
            "shape",
            "/// the vector [OFW-4.93 lv1 a:0xC24] reads it",
            None,
        ),
        (
            "shape",
            "/// [OFW v4.93 (0x2EA35C)]",
            Some(Fault::MalformedFirmwareTag(
                "OFW v4.93 (0x2EA35C)".to_string(),
            )),
        ),
        (
            "shape",
            "/// [OFW-4.93 lv1 0x2EA35C]",
            Some(Fault::MalformedFirmwareTag(
                "OFW-4.93 lv1 0x2EA35C".to_string(),
            )),
        ),
        (
            "shape",
            "/// [OFW-4.93 lv1]",
            Some(Fault::MalformedFirmwareTag("OFW-4.93 lv1".to_string())),
        ),
        (
            "shape",
            "/// [OFW-4.93 lv1 a:0x2EA35C s:handler]",
            Some(Fault::MalformedFirmwareTag(
                "OFW-4.93 lv1 a:0x2EA35C s:handler".to_string(),
            )),
        ),
    ];
    for rule in [
        "version",
        "core image",
        "module",
        "address",
        "window",
        "shape",
    ] {
        let rows: Vec<_> = cases.iter().filter(|(r, _, _)| *r == rule).collect();
        assert!(
            rows.iter().any(|(_, _, f)| f.is_none()),
            "{rule} has no pass"
        );
        assert!(
            rows.iter().any(|(_, _, f)| f.is_some()),
            "{rule} has no fail"
        );
    }
    for (rule, line, expected) in cases {
        let (valid, got) = fault(line);
        assert_eq!(&got, expected, "{rule}: {line}");
        assert_eq!(valid, usize::from(expected.is_none()), "{rule}: {line}");
    }
}

#[test]
fn a_firmware_fact_cited_in_prose_is_a_fault_and_its_tag_form_is_not() {
    let plain = |form: &str| Some(Fault::PlainFirmwareCitation(form.to_string()));
    // Each form: one line that passes it and one that fails it.
    let cases: &[(&str, &str, Option<Fault>)] = &[
        (
            "stock",
            "/// The shipped 4.93 `lv2_kernel.self` sets up one record per thread.",
            None,
        ),
        (
            "stock",
            "/// The stock 4.93 `lv2_kernel.self` sets up one record per thread.",
            plain("stock 4.93 `lv2_kernel.self`"),
        ),
        (
            "stock",
            "/// identical to the stock 4.93 image, except one word.",
            plain("stock 4.93 image,"),
        ),
        (
            "reading",
            "/// The kernel's wrapper [OFW-4.93 lv2_kernel a:0x800000000000FC54] passes r3.",
            None,
        ),
        (
            "reading",
            "/// Firmware reading: the 4.93 kernel's wrapper passes r3.",
            plain("Firmware reading:"),
        ),
        (
            "version and image",
            "/// The 4.93 kernel's `sc` [OFW-4.93 lv2_kernel a:0x800000000005dcf4] passes r5.",
            None,
        ),
        (
            "version and image",
            "/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record.",
            None,
        ),
        (
            "version and image",
            "/// The 4.93 kernel's `sc` at 0x800000000005dcf4 passes r5.",
            plain("4.93 kernel's"),
        ),
        (
            "version and image",
            "/// lv1's 4.93 handler (0x2EA35C) reads three inputs.",
            plain("lv1's 4.93"),
        ),
        (
            "image and address",
            "/// the kernel at [OFW-4.93 lv2_kernel a:0x8000000000007D2C] calls lv1",
            None,
        ),
        (
            "image and address",
            "/// the call from kernel 0x7d2c reaches the handler",
            plain("kernel 0x7d2c"),
        ),
        (
            "image and address",
            "/// the handler in lv1 at 0x2EA35C reads three inputs",
            plain("lv1 at 0x2EA35C"),
        ),
        (
            "image and address",
            "[\"fw 1.00, size 10, image 0x0000000000000001\"]",
            None,
        ),
        (
            "at version",
            "/// 1. the size, at [OFW-4.93 lv1 a:0x2DEF60];",
            None,
        ),
        (
            "at version",
            "/// 1. the size, at 4.93 0x2DEF60;",
            plain("at 4.93 0x2DEF60;"),
        ),
    ];
    for form in [
        "stock",
        "reading",
        "version and image",
        "image and address",
        "at version",
    ] {
        let rows: Vec<_> = cases.iter().filter(|(f, _, _)| *f == form).collect();
        assert!(
            rows.iter().any(|(_, _, f)| f.is_none()),
            "{form} has no pass"
        );
        assert!(
            rows.iter().any(|(_, _, f)| f.is_some()),
            "{form} has no fail"
        );
    }
    for (form, line, expected) in cases {
        let (_, faults) = scan_line(line);
        let got: Vec<_> = faults
            .into_iter()
            .filter(|f| matches!(f, Fault::PlainFirmwareCitation(_)))
            .collect();
        assert_eq!(
            got,
            expected.iter().cloned().collect::<Vec<_>>(),
            "{form}: {line}"
        );
    }
}

#[test]
fn the_firmware_table_maps_each_pup_version_to_the_module_paths_it_ships() {
    let pups = "pup_sha256\tfw\tsize_bytes\naa\t4.93\t1\nbb\t1.50\t1\ncc\t4.93\t1\n";
    let modules = "pup_sha256\tmodule\tsites\naa\tsys/external/liblv2.sprx\tnone\n\
                   cc\tvsh/module/vsh_plugin.sprx\tnone\n";
    let table = load_firmware_modules(pups, modules);
    assert_eq!(table.keys().collect::<Vec<_>>(), ["1.50", "4.93"]);
    assert!(table["1.50"].is_empty());
    assert_eq!(
        table["4.93"].iter().collect::<Vec<_>>(),
        ["sys/external/liblv2.sprx", "vsh/module/vsh_plugin.sprx"]
    );
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
         for the AltiVec manuals; I/II/III/App-prefixed for the Power ISA), \
         `[WEB-KEY s:SECTION]` with a web source alias in place of a raw psdevwiki or \
         Wayback link, or `[OFW-<version> <image> a:<addr>[-<addr>]]` with a version of \
         {PUP_TABLE}, an image that version ships, and a forward hexadecimal window:\n{}",
        violations.len(),
        violations.concat()
    );
}
