//! The LV1 archive: the text tables under `docs/lv1/` that describe the
//! lv1 hypercalls each LV2 kernel makes and the repository nodes it
//! reads, and the rules they obey.
//!
//! Text in, text out: nothing here opens a file. The `lv1_archive`
//! integration test does the I/O for the generated files: it fails when
//! they drift, and writes them under `CELLGOV_REGENERATE=1`.
//!
//! The table machinery and the firmware, PUP and kernel tables come
//! from `cellgov_lv2_archive`. That crate never depends on this one.
//! The runtime never calls this crate.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

mod hypercall;
mod repository;
mod spec;
mod sql;

pub use hypercall::{
    census_answered, census_contradictions, hypercall_family, hypercall_rows, hypercall_site_rows,
    hypercall_site_tsv, hypercall_tsv, hypercall_unresolved_rows, hypercall_unresolved_tsv,
    HypercallCensus, HypercallRow, HypercallSiteRow, HypercallState, HypercallUnresolvedRow,
    Lv1Entry,
};
pub use repository::{
    replace_repository_reads, repository_read_rows, repository_read_rows_of, repository_read_tsv,
    repository_rows, repository_tsv, MixedRepositoryRead, RepositoryReadRow, RepositoryRow,
    RepositoryTableError,
};
pub use spec::{
    files, manifest, GATE, HYPERCALL, HYPERCALL_FAMILIES, HYPERCALL_GATE, HYPERCALL_LEAVES,
    HYPERCALL_LV1_ENTRIES, HYPERCALL_REGENERATE, HYPERCALL_SITE, HYPERCALL_STATES,
    HYPERCALL_UNRESOLVED, REGENERATE, REPOSITORY, REPOSITORY_GATE, REPOSITORY_LOGGED_FIRMWARE,
    REPOSITORY_READ, REPOSITORY_READ_STATES, REPOSITORY_REGENERATE, REPOSITORY_SOURCES,
    REPOSITORY_UNOBSERVED, SCHEMA_VERSION, SHARED_TABLES, TABLES,
};
pub use sql::{build_sql, schema_sql, SHARED_TABLE_DIR};
