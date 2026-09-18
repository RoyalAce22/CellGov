# LV2 archive

<!-- Rendered from `cellgov_lv2::archive` by
`cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate`.
Do not edit by hand: `committed_archive_matches_generator` fails on drift. -->

Text tables describing the LV2 syscall surface and CellGov's handling
of it. The tables are the archive. SQLite is the query engine, built
from them on demand by `build.sql` and never committed: a binary
would pass every diff-based review unseen, is not byte-stable, and
would add its full size to public history on every change.

Nothing reads these tables at dispatch. They describe the code; the
code does not consult them.

## Manifest

Every file in this directory has a row here, and a guard fails when
the directory and this table disagree.

| File | Owner class | Regenerate | Gate |
| --- | --- | --- | --- |
| `README.md` | generated | `cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate` | `committed_archive_matches_generator` |
| `arm.tsv` | generated | `cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate` | `committed_archive_matches_generator` |
| `behavior.tsv` | curated | written by hand | `behavior_rows_cover_the_handled_surface` |
| `build.sql` | generated | `cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate` | `committed_archive_matches_generator` |
| `route.tsv` | generated | `cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate` | `committed_archive_matches_generator` |
| `schema.sql` | generated | `cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate` | `committed_archive_matches_generator` |

## Owner classes

Every table names exactly one.

| Class | Meaning |
| --- | --- |
| extracted | Written only by the census emitter from firmware bytes, under anchor discipline. |
| generated | Rendered from code by a regenerate test; a drift gate fails when the committed copy is stale. |
| curated | CellGov's own claims, loader-validated, with provenance on every row. |
| attributed | Community or non-public names, with a source on every row, never merged into extracted rows. |

Anything specific to one operator or checkout, such as key-vault
coverage or data derived from another runner, is a local overlay
under `vfs/.cellgov/`, never a file here.

## Handling

`route.tsv` has one row per LV2 syscall slot, and `arm.tsv` one row
per dispatch arm with the slots that reach it. Of the 1024 slots:

| Route | Slots | Meaning |
| --- | ---: | --- |
| `typed` | 118 | Classifies to a typed request arm. |
| `routed` | 25 | Reaches a dedicated arm inside `Unsupported`. |
| `null_backend` | 879 | The honest traced `CELL_ENOSYS` refusal through the generic arm. |
| `runtime_fast_path` | 2 | Answered by the runtime's timer path; never classified or dispatched. |

The routing-layer guarantee -- an unhandled syscall returns
`CELL_ENOSYS` through the null backend, never a fabricated success --
holds for the whole surface. How much real LV2 behavior each handled
arm reproduces is a per-arm property, tagged in `arm.tsv`:

| Tag | Meaning |
| --- | --- |
| `modeled` | Full modeled state and ABI. |
| `partial-state` | ABI faithful; some kernel-visible state simplified or omitted. |
| `abi-only` | Plausible return value with little or no backing state. |
| `null-backend` | Honest `CELL_ENOSYS`-class refusal with a logged diagnostic. |

The tags are reviewed claims: table membership is probe-gated against
dispatch, but a tag's accuracy rests on the review recorded in each
arm's rustdoc under `crates/cellgov_lv2/src/host/`, not on a machine
check.

## Behavior

`behavior.tsv` is curated: one row per typed or routed ordinal, written
by hand, saying what the modelled behaviour rests on and what pins it.
What the arm does lives in its rustdoc; nothing is restated here.

| Column | Meaning |
| --- | --- |
| `packet` | The field an ordinal multiplexes on (`cmd`, `package_id`, `pkg_id`), or `none` for a flat ordinal. |
| `same_as` | The ordinal whose arm this one shares, or `none`; symmetric. |
| `selector_slot` | The argument register the arm dispatches on (`r3`..`r10`), or `none`. |
| `provenance_kind` | `citation` (an official document), `firmware_reading` (a call site or wrapper in the installed firmware), `console_capture` (a fixture captured on a console), `non_public` (a fact from a source that cannot be cited), `unestablished` (nothing fixes it). |
| `provenance_ref` | The citation as `DOC-KEY:p:N`, the reading or capture as a locator, or `none` for the last two kinds. |
| `witness` | A non-ignored test that needs no corpus, as `path:function`, or `none` against a committed baseline that only shrinks. |
| `exception` | `fabricated_success` for an arm whose zero-argument probe answers `CELL_OK` and records an invariant break: the call is acknowledged, not modelled. `none` otherwise, and the gate refuses a `none` on an arm that probes that way. |
| `arm_source` | The file holding the arm's implementation. |

The gate (`behavior_rows_cover_the_handled_surface` and its siblings in
`cellgov_lv2::archive`) fails when the rows and the typed or routed
surface disagree, when a witness is not a non-ignored test in a
corpus-free crate, when a row without a witness is not in the
baseline, when `arm_source` does not hold the arm, when a reference
does not fit its kind, and when a `fabricated_success` row does not
fabricate one.

## Table rules

The loader in `cellgov_lv2::archive` refuses a table that breaks any
of these. `build.sql` re-checks only the column types, the enumerated
labels and the foreign keys (STRICT tables, CHECK constraints,
`PRAGMA foreign_keys`); the sqlite3 import pads a short row, drops a
long row's extras with a warning, and reads a malformed integer as a
number, so cell count and cell shape are the loader's alone:

- a header row naming the columns, then one row per line, each
  line ending in a line feed;
- rows sorted by the table's key, with no key repeated;
- no empty cell: a column that takes no value carries `none`;
- ASCII only; no tab, carriage return or leading `"` inside a cell;
- every cell matches its column's kind: a decimal integer, an
  identifier of letters, digits and `_`, an ascending comma-joined
  integer list, one of an enumerated set of labels, or a locator of
  letters, digits and `_ . / : @ + -`.

## Querying

Built and verified with sqlite3 3.53.0; the `.import`
options `build.sql` uses are absent from older shells. From this
directory:

    sqlite3 lv2.db < build.sql
    sqlite3 lv2.db "SELECT ordinal, arm, fidelity FROM handling WHERE route = 'typed'"
    sqlite3 lv2.db "SELECT ordinal, arm, provenance_kind FROM authority WHERE witness IS NULL"

`schema.sql` holds the tables and the join-only views (`handling`
over route and arm, `authority` over behavior, route and arm);
`build.sql`
reads it, imports each table through a staging table, and stops at
the first error.
