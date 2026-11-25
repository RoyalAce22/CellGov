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

Archive schema version: **2**. `sql/schema.sql` records the
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
resolved. A site is also unresolved when control can enter a word between
its `li r11` and `sc` in another way. These ways are the ELF entry, an
interrupt vector, and an indirect branch to an address that the kernel
names. Both tables hold derived facts only, never kernel bytes.

A resolved site states one fact: on each path that the census models
from an entry it records, r11 holds the site's number at the `sc`.
When the census cannot prove that, it leaves the site unresolved. That
loss is correct behaviour, so a number can appear in a kernel only at
unresolved sites. A resolved site with another number on a modelled
path is a defect, and so is a missed entry or a dropped lost mark. The
census does not state that a resolved site is reachable, or anything
about code outside the scanned segments. What a kernel run observes
never resolves a site.

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

`name_source` tells which source gave the name:

- `header`: `lv1call.h`, in its Linux copy or its OtherOS++ copy. The
  OtherOS++ copy names some numbers only `undocumented_function_N`.
- `psdevwiki`: psdevwiki's list of hypercalls.
- `cellgov`: CellGov's reading of the call's lv1 handler in the 4.93
  firmware. The call table cites the handler of each such name.

A psdevwiki or CellGov name replaces the header's placeholder. The
family comes from the `name` column, so a psdevwiki or CellGov name
decides the family of its call. The `console` family holds the logical
console calls, 105, 106, 107, 108, 109 and 231, whether or not
the table has a row for the number. No header names them: the headers
give 105 to 109 only `undocumented_function_N`, and their psdevwiki
names put them in `console`. A call whose name is still the header's
placeholder is `undocumented`, and a number with no name is `unnamed`.

A number that neither header lists has no header row and no
placeholder. `UNLISTED_ALIASES` in `cellgov_ps3_abi::lv1::call` gives a
psdevwiki or CellGov name to some of these numbers:
89, 137, 195, 196, 230 and 231. Each of them takes its name from that alias.
Every other number that neither header lists has `none` as its name and
its `name_source`. A number that neither header lists has `none` as its
`inputs` and `outputs`.

`lv1_493` is `handler`, `invalid` or `past_table`. It comes from
`cellgov_ps3_abi::lv1::table`, which holds the console capture
`tests/micro/lv1_hypercall_table`. A `past_table` number
(0 numbers) is not below the table's entry
count, 256. The state is `answered`
(43 numbers) or `null_backend`: no handler answers
the number.

lv1's vector answers a `past_table` number before any handler runs. It
returns lv1's unknown-hypercall status, `LV1_NOT_IMPLEMENTED`, to a
call from either processor state. CellGov gives that answer, whatever
the row's state, at each firmware whose lv1 image shows the entry count:
1.94, 3.15, 3.41, 3.70 and 4.93. At every other firmware:

- CellGov refuses a problem-state call to the number, and the refusal
  stops the run.
- A supervisor-state call gets its handler's answer, or the null backend
  stops the run.

For a number below the entry count, the null backend at 4.93 answers
lv1's invalid-entry status for an `invalid` number
(17 numbers). It stops the run for a `handler`
number. At every other firmware it stops the run.

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

`tables/repository.tsv` has 90 rows. Each row holds a
node's key, its two values, the firmware CellGov serves its values to
and its source. Each row is one of these nodes:

- a node that `cellgov_ps3_abi::lv1::repository::NODES` serves;
- a node that lv1's storage registration writes and a logged boot reads
  with an answer. Its values come from
  `cellgov_ps3_abi::lv1::storage_registration`, the registration after lv1
  registered each device of the captured bus table. The hypervisor
  answers such a node only after the registration, so a logged refusal
  of it needs no row.

The source can be of another firmware: the psdevwiki dump is of 3.41. A read at a
firmware that no row of its key names is a refusal, except for a key of
`cellgov_ps3_abi::lv1::repository::ABSENT_NODES`. A capture that walked the
whole repository list shows that key has no node at that firmware, so the
hypervisor answers it with lv1's no-node status (`NO_NODE_R3`) and no row
names it. `tables/repository_read.tsv` calls such a read `answered`, the
same word it uses for a served node; a read with no row in
`tables/repository.tsv` is one of these. `v2` reads
`unobserved` where the source does not observe the second
value. The hypervisor answers such a node only at the read sites that a
static analysis of the kernel shows discard the second value. A read
from any other site is a refusal. These cases fail the generator:

- a row that names a firmware whose logged boot does not read the node;
- an absent key for a firmware whose logged boot does not read it;
- an absent key of a bus or a partition whose nodes the walk's capture
  does not keep;
- an absent key that lv1's storage registration writes;
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
