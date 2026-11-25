# LV1 archive

<!-- Rendered from `cellgov_lv1_archive` by
`CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator`.
Do not edit by hand: `committed_archive_matches_generator` fails on drift. -->

Text tables describing the lv1 interface that LV2 kernels use: the
hypercalls each kernel makes and the repository nodes it reads, and
how CellGov's hypervisor answers them. The tables are the archive.
SQLite is the query engine, built from them on demand by
`sql/build.sql` and never committed, for the reasons the
[LV2 archive](../lv2/README.md) gives.

Nothing reads these tables at dispatch. They describe the code; the
code does not consult them.

## Directory layout

- `tables/` holds the archive's TSV tables.
- `sql/` holds the generated schema and database-build scripts.

The firmware, PUP and kernel tables are the LV2 archive's:
`firmware.tsv`, `pup.tsv` and `kernel.tsv` below name the files under
`../lv2/tables/`, and `sql/build.sql` imports the first two
from there. Run the SQLite build command from this directory so its
relative paths resolve.

Archive schema version: **1**. `sql/schema.sql` records the
same value in SQLite's `user_version`, so a column change cannot pass as
an unchanged archive.

## Manifest

Every file in this directory has a row here, and a guard fails when
the directory and this table disagree.

| File | Owner class | Regenerate | Gate |
| --- | --- | --- | --- |
| `README.md` | generated | `CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator` | `committed_archive_matches_generator` |
| `sql/build.sql` | generated | `CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator` | `committed_archive_matches_generator` |
| `sql/schema.sql` | generated | `CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator` | `committed_archive_matches_generator` |
| `tables/hypercall.tsv` | generated | `CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator` | `committed_archive_matches_generator` |
| `tables/hypercall_site.tsv` | extracted | `CELLGOV_REGENERATE=1 cargo test -p cellgov_cli --features installed-firmware-tests --test lv2_census_firmware firmware_reextracts_byte_identical_hypercall_rows` | `hypercall_site_rows_are_well_formed` |
| `tables/hypercall_unresolved.tsv` | extracted | `CELLGOV_REGENERATE=1 cargo test -p cellgov_cli --features installed-firmware-tests --test lv2_census_firmware firmware_reextracts_byte_identical_hypercall_rows` | `hypercall_site_rows_are_well_formed` |
| `tables/repository.tsv` | generated | `CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator` | `committed_archive_matches_generator` |
| `tables/repository_read.tsv` | extracted | `CELLGOV_REGENERATE=1 cargo test -p cellgov_cli --features installed-firmware-tests --test repository_census_firmware` | `repository_rows_are_well_formed` |

## Owner classes

Every table names exactly one.

| Class | Meaning |
| --- | --- |
| extracted | Written only by the census emitter from firmware bytes, under anchor discipline. |
| generated | Rendered from code; a drift gate fails when the committed copy is stale, and under CELLGOV_REGENERATE=1 writes it. |
| curated | CellGov's own claims, loader-validated, with provenance on every row. |
| attributed | Community or non-public facts, with a source on every row, never merged into extracted rows. |

## Kernel hypercalls

`tables/hypercall_site.tsv` groups each kernel's LV1 hypercall sites by
PUP hash and by the constant that every path to the site loads into r11.
A site is a word that is exactly `sc 1`, 0x44000022. A data word that
decodes as `sc 1` but has a nonzero reserved field is not a site. The
table records the real address of each site: real mode ignores EA bits 0
and 1, so a kernel address loses its top bit. `tables/hypercall_unresolved.tsv`
has one row for every scanned kernel; `sites` is `none` when every site
resolved. Both tables hold derived facts only, never kernel bytes.

`cellgov dev hypercall-census` writes one kernel's rows of both tables.
`CELLGOV_REGENERATE=1 cargo test -p cellgov_cli --features installed-firmware-tests --test lv2_census_firmware firmware_reextracts_byte_identical_hypercall_rows` runs it over every `kernel.tsv` kernel in the
installed firmware and rewrites both; without that flag the same test
fails when a re-extraction differs. `hypercall_site_rows_are_well_formed` checks the
coverage rules without reading external data.

`tables/hypercall.tsv` has 124 rows, one per number a
scanned kernel calls or CellGov's hypervisor answers. Each row joins the
call table in `cellgov_ps3_abi::lv1::call` (the name and the input and
output register counts), CellGov's family for the call and the work item
that answers that family, the firmware versions whose kernel calls the
number with its site count in the newest of them, its entry in lv1's
live 4.93 hypercall table, and the state.

`lv1_493` is `handler`, `invalid` or `past_table`. It comes from
`cellgov_ps3_abi::lv1::table`, which holds the console capture
`tests/micro/lv1_hypercall_table`. The state is `answered`
(17 numbers) or `null_backend`: no handler answers
the number. At 4.93 the null backend answers lv1's invalid-entry status
for an `invalid` number (17 numbers) and stops the run
for a `handler` number. At every other firmware it stops the run.

The state comes from the handler each hypercall group of `cellgov_lv1`
registers. A change that registers a handler need not regenerate this
table, so a newly answered number can still read `null_backend` until
the next regeneration. `committed_archive_matches_generator` allows that lag. It refuses a number
that the table calls `answered` when no handler answers it.

## Hypervisor repository

`tables/repository_read.tsv` logs the repository reads, hypercall 91,
that each logged firmware's kernel makes when it boots from its entry
under CellGov's hypervisor: 1.50, 1.94, 3.70, 4.93. Each row is
one key of one boot: the partition and the four raw key words, the key
as text, the position of the boot's first read of it, and whether the
hypervisor answered or refused it. In the text, a named word's index
follows a `+` and a numeric word reads in hex, so `bi.pu+1` and
`bi.pu.0x1` are two keys. A refused read stops the boot, so it is the
boot's last; the refused nodes are `bi.rgntotal`.

`cellgov dev repository-census` writes one kernel's rows.
`CELLGOV_REGENERATE=1 cargo test -p cellgov_cli --features installed-firmware-tests --test repository_census_firmware` boots every logged firmware's stored kernel
and rewrites the table; without that flag the same test fails when a
re-extraction differs. `repository_rows_are_well_formed` checks the logged set, the
text of each key, the order of each boot's reads and the source of each
served node without reading external data.

`tables/repository.tsv` has 13 rows, one per node that
`cellgov_ps3_abi::lv1::repository::NODES` serves: its key, its two
values, the firmware CellGov serves its values to and its source. The
source can be of another firmware: the psdevwiki dump is of 3.41. A read at a
firmware that no row of its key names is a refusal. `v2` reads
`unobserved` where the source does not observe the second
value. The hypervisor answers such a node only at the read sites that a
static analysis of the kernel shows discard the second value. A read
from any other site is a refusal. These cases fail the generator:

- a row that names a firmware whose logged boot does not read the node;
- a logged read that the table no longer agrees with;
- a logged refusal of a node that the hypervisor answers only at the
  read sites it lists: the read's site is not one of them, or the log is
  stale.

`cellgov dev repository-census` refuses a boot that answers one key at
one read and refuses it at another, because one row holds one state per
key.

## Table rules

The loader in `cellgov_lv2_archive` refuses a table of this archive on
the rules the LV2 archive's [table rules](../lv2/README.md#table-rules)
list.

## Querying

Built and verified with sqlite3 3.53.0. From this directory:

    sqlite3 lv1.db < sql/build.sql
    sqlite3 lv1.db "SELECT number, name, state FROM hypercall WHERE lv1_493 = 'handler'"
    sqlite3 lv1.db "SELECT fw, node, state FROM repository_read JOIN pup USING (pup_sha256)"

`sql/schema.sql` holds the firmware and PUP tables and then this
archive's tables. `sql/build.sql` reads it, imports each table through a
staging table, and stops at the first error.
