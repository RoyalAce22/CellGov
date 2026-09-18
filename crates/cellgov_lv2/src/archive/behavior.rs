//! The cell shapes `behavior.tsv` carries beyond what the loader checks.

/// A witness: a non-ignored test, named by file path and function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Witness<'a> {
    /// The test file, relative to the workspace root.
    pub path: &'a str,
    /// The test function.
    pub function: &'a str,
}

/// Crates whose tests need no corpus, so a witness may live in them.
pub const WITNESS_CRATES: &[&str] = &[
    "crates/cellgov_lv2/",
    "crates/cellgov_core/",
    "crates/cellgov_explore/",
];

/// Parse a `path:function` witness cell.
///
/// Returns `None` when:
/// - the cell has no `:`;
/// - either half is empty;
/// - the path has a `..` component, which would step out of the crate
///   the prefix names;
/// - the path is outside [`WITNESS_CRATES`].
pub fn parse_witness(cell: &str) -> Option<Witness<'_>> {
    let (path, function) = cell.rsplit_once(':')?;
    let in_crate = WITNESS_CRATES.iter().any(|c| path.starts_with(c));
    let climbs = path.split('/').any(|component| component == "..");
    (!path.is_empty() && !function.is_empty() && in_crate && !climbs)
        .then_some(Witness { path, function })
}

/// The official-document keys a `citation` provenance may name.
pub const DOC_KEYS: &[&str] = &[
    "PowerISA-3.1",
    "PPC-Book1",
    "PPC-Book2",
    "PPC-Book3",
    "CBE-Handbook",
    "CBEA",
    "SPU-ISA",
    "AltiVec-PEM",
    "AltiVec-PIM",
];

/// Parse a `DOC-KEY:p:N` citation reference into `(key, page)`.
///
/// The page keeps the printed-page form a citation tag in the Rust
/// sources carries, so a reviewer can resolve every reference.
pub fn parse_citation(cell: &str) -> Option<(&str, &str)> {
    let (key, page) = cell.split_once(":p:")?;
    let page_form = page
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    (DOC_KEYS.contains(&key) && !page.is_empty() && page_form).then_some((key, page))
}

/// Whether `reference` has the shape `kind` needs.
pub fn provenance_ref_fits(kind: &str, reference: Option<&str>) -> bool {
    match kind {
        "citation" => reference.is_some_and(|r| parse_citation(r).is_some()),
        "firmware_reading" | "console_capture" => reference.is_some(),
        "non_public" | "unestablished" => reference.is_none(),
        _ => false,
    }
}

/// The arm identifier, lowercased, with a trailing ordinal dropped.
///
/// A [`foldable`] source that defines the arm's dispatch function
/// contains this token: `MemoryContainerCreate324` folds to
/// `memorycontainercreate`, inside `dispatch_memory_container_create`.
pub fn arm_token(arm: &str) -> String {
    arm.trim_end_matches(|c: char| c.is_ascii_digit())
        .to_ascii_lowercase()
}

/// `text` lowercased with every `_` removed.
///
/// A caller searches this form for an [`arm_token`].
pub fn foldable(text: &str) -> String {
    text.to_ascii_lowercase().replace('_', "")
}

#[cfg(test)]
#[path = "tests/behavior_tests.rs"]
mod tests;
