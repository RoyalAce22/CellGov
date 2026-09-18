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
| `conflicts.tsv` | generated | `cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate` | `committed_archive_matches_generator` |
| `name.tsv` | attributed | `cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate_cellgov_names` (the `cellgov` rows) | `cellgov_name_rows_match_the_macro` |
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

## Names

An ordinal is extracted: the kernel's dispatch table fixes it. A name
is attributed: no firmware byte carries one, so `name.tsv` records
every name a committed source gives an ordinal as one row against
that source, and an ordinal no source names has no row. That is a
normal state, not a gap. Where the sources disagree, every candidate
stays and none is preferred; the disagreement is the information.
621 slots carry a name.

| Source | Rows | Meaning |
| --- | ---: | --- |
| `psdevwiki` | 618 | The name cell of psdevwiki's LV2 Functions and Syscalls table, taken only where it is one plain identifier and not a stub ending in `_`; `ref` is the page. |
| `psl1ght` | 240 | `sys_` and the lowercased rest of a `SYSCALL_` token in PSL1GHT's `lv2/syscalls.h`; `ref` is the header and the token. |
| `cellgov` | 144 | The name field of the `lv2_syscalls!` macro; `ref` is the constant. CellGov's own vocabulary, rendered, never hand-copied. |
| `non_public` | 0 | A name known only from material that cannot be cited; `ref` is `none`. |

`packet` is `none` for a name that applies to the whole ordinal.
`fw_from` and `fw_to` bound the firmware era the source gives the name
for, or read `none` where it gives none. The `cellgov` rows are
rendered from the name field of the `lv2_syscalls!` macro in
`cellgov_ps3_abi::lv2::syscall`: `cellgov_name_rows_match_the_macro` fails when the
committed rows differ, `cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate_cellgov_names` rewrites them and leaves
every other row as it is, and a macro entry with no name field (462)
renders no row. A row's `ref` has to be what its source's meaning
promises, and a `psl1ght` name has to be the transform of the token
its `ref` names; `every_name_row_fits_its_source` fails otherwise.

Another runner's syscall names are not a source: they never enter a
tracked file. An operator may derive them from their own checkout
into an overlay under `vfs/.cellgov/` and join it into their own
build. An exported function that reaches an ordinal is not a name
for it either; the reach tables report exports beside names.

### Conflicts

`conflicts.tsv` holds the rows of `name.tsv` whose slot carries more
than one distinct name: `spelling` when the names are one identifier
under different leading underscores, `name` otherwise.
31 slots:

| Slot | Disagreement | Names (sources) |
| --- | --- | --- |
| 22 | name | `sys_process_exit` (cellgov), `sys_process_exit2` (psdevwiki) |
| 52 | spelling | `_sys_ppu_thread_create` (cellgov), `sys_ppu_thread_create` (psdevwiki) |
| 95 | spelling | `_sys_lwmutex_create` (psdevwiki), `sys_lwmutex_create` (cellgov) |
| 96 | spelling | `_sys_lwmutex_destroy` (psdevwiki), `sys_lwmutex_destroy` (cellgov) |
| 97 | spelling | `_sys_lwmutex_lock` (psdevwiki), `sys_lwmutex_lock` (cellgov) |
| 98 | spelling | `_sys_lwmutex_unlock` (psdevwiki), `sys_lwmutex_unlock` (cellgov) |
| 99 | spelling | `_sys_lwmutex_trylock` (psdevwiki), `sys_lwmutex_trylock` (cellgov) |
| 150 | name | `sys_raw_spu_create_interrupt_tag` (psdevwiki), `sys_spu_create_interrupt_tag` (psl1ght) |
| 151 | name | `sys_raw_spu_set_int_mask` (psdevwiki), `sys_spu_set_int_mask` (psl1ght) |
| 152 | name | `sys_raw_spu_get_int_mask` (psdevwiki), `sys_spu_get_int_mask` (psl1ght) |
| 153 | name | `sys_raw_spu_set_int_stat` (psdevwiki), `sys_spu_set_int_stat` (psl1ght) |
| 154 | name | `sys_raw_spu_get_int_stat` (psdevwiki), `sys_spu_get_int_stat` (psl1ght) |
| 158 | name | `sys_spu_image_close` (psdevwiki), `sys_spu_image_import` (cellgov) |
| 160 | name | `sys_raw_spu_create` (psdevwiki), `sys_spu_create` (psl1ght) |
| 161 | name | `sys_raw_spu_destroy` (psdevwiki), `sys_spu_destroy` (psl1ght) |
| 163 | name | `sys_raw_spu_read_puint_mb` (psdevwiki), `sys_spu_read_puint_mb` (psl1ght) |
| 190 | name | `sys_spu_thread_write_ls_mb` (cellgov), `sys_spu_thread_write_spu_mb` (psdevwiki, psl1ght) |
| 196 | name | `sys_raw_spu_set_spu_cfg` (psdevwiki), `sys_spu_set_spu_cfg` (psl1ght) |
| 197 | name | `sys_raw_spu_get_spu_cfg` (psdevwiki), `sys_spu_get_spu_cfg` (psl1ght) |
| 199 | name | `sys_raw_spu_recover_page_fault` (psdevwiki), `sys_spu_recover_page_fault` (psl1ght) |
| 341 | spelling | `_sys_memory_container_create` (psdevwiki), `sys_memory_container_create` (cellgov) |
| 362 | name | `sys_mmapper_allocate_memory_from_container` (psdevwiki), `sys_mmapper_allocate_shared_memory_from_container` (cellgov) |
| 480 | spelling | `_sys_prx_load_module` (cellgov), `sys_prx_load_module` (psdevwiki, psl1ght) |
| 481 | spelling | `_sys_prx_start_module` (cellgov), `sys_prx_start_module` (psdevwiki, psl1ght) |
| 482 | spelling | `_sys_prx_stop_module` (cellgov), `sys_prx_stop_module` (psdevwiki, psl1ght) |
| 483 | spelling | `_sys_prx_unload_module` (cellgov), `sys_prx_unload_module` (psdevwiki, psl1ght) |
| 484 | spelling | `_sys_prx_register_module` (cellgov), `sys_prx_register_module` (psdevwiki, psl1ght) |
| 486 | spelling | `_sys_prx_register_library` (cellgov), `sys_prx_register_library` (psdevwiki, psl1ght) |
| 494 | spelling | `_sys_prx_get_module_list` (cellgov), `sys_prx_get_module_list` (psdevwiki, psl1ght) |
| 497 | spelling | `_sys_prx_load_module_on_memcontainer` (cellgov), `sys_prx_load_module_on_memcontainer` (psdevwiki, psl1ght) |
| 617 | name | `sys_storage_check_region_acl` (psdevwiki), `sys_storage_get_region_acl` (psl1ght) |

### Names only CellGov carries

Names no other committed source gives. They are CellGov's own
vocabulary and nothing here corroborates them; the constant's rustdoc
carries whatever else is known. 3 slots:

| Slot | Name | Constant |
| --- | --- | --- |
| 26 | `_sys_process_exit2` | `PROCESS_EXIT2` |
| 27 | `sys_process_spawns_a_self2` | `PROCESS_SPAWNS_A_SELF2` |
| 512 | `sys_hid_manager_is_process_permission_root` | `HID_IS_ROOT` |

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

A table's key may hold a nullable column (`name.packet`): `none`
sorts as the four letters, and two rows with `none` there and the
same other key cells repeat the key.

## Querying

Built and verified with sqlite3 3.53.0; the `.import`
options `build.sql` uses are absent from older shells. From this
directory:

    sqlite3 lv2.db < build.sql
    sqlite3 lv2.db "SELECT ordinal, arm, fidelity FROM handling WHERE route = 'typed'"
    sqlite3 lv2.db "SELECT ordinal, arm, provenance_kind FROM authority WHERE witness IS NULL"
    sqlite3 lv2.db "SELECT name, source FROM name WHERE ordinal = 190"

`schema.sql` holds the tables and the join-only views (`handling`
over route and arm, `authority` over behavior, route and arm; none
joins `name`, so no view reads as giving an ordinal its name);
`build.sql` reads it, imports each table through a staging table, and
stops at the first error. A table whose key holds a nullable column
(`name`, `conflicts`) is `UNIQUE` rather than `PRIMARY KEY` in SQL,
where a `STRICT` key column could not be null; the loader alone
refuses a repeated key with a `none` in it.
