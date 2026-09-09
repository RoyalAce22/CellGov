# Title manifest guide

A title manifest is a single TOML file under
`title_manifests/` that tells `cellgov_cli` how to boot one
PS3 title: where its EBOOT lives, what its boot harness should
treat as the stopping point, and which pieces of content the
boot path needs visible through the LV2 VFS.

A manifest describes the boot input for static-recompilation
analysis. It does not configure gameplay.

## Where they live and how they are discovered

- Path: `title_manifests/<content_id>.toml`. The filename
  is convention only; the loader keys off the `content_id`
  field inside the file.
- The registry at startup scans this directory
  (`cellgov_cli`'s `DEFAULT_TITLE_REGISTRY_DIR`). Adding a
  manifest is a single-file commit; the schema is enforced by
  the loader and no Rust change is required.
- CLI lookup keys:
  - `--title <short_name>`
  - `--content-id <SERIAL>`
  - `--title-manifest <path>` (bypass the registry)

## Schema overview

The file has one required `[title]` block, one required
`[checkpoint]` block, and five optional blocks (`[source]`,
`[rsx]`, `[content]`, `[[fs.mounts]]`, `[[bench.matrix]]`). The
TOML parser runs with `deny_unknown_fields`; a typo in a key name
surfaces as a load error.

The matrix rendered into [../titles.md](../docs/titles.md) is
generated from this directory plus per-title fixture summaries
via `cellgov dev titles-gen`; the system software renders on
[../firmware.md](../docs/firmware.md) from the same run.

### `[title]` (required)

| Field              | Type     | Required | Notes                                                                                                                                                                                                                                |
| ------------------ | -------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `content_id`       | string   | usually  | PSN serial. Disc-ISO titles use `BCES.../BLES.../BLUS...`; PSN titles use `NPUA.../NPEA...`. Omittable only by a `manifest-relative` title, which has no PSN identity; the loader then uses the manifest's own directory name.        |
| `short_name`       | string   | yes      | Kebab-cased lookup label used by `--title`. Pick something short and stable; tests reference it.                                                                                                                                     |
| `display_name`     | string   | yes      | Human title for matrix output.                                                                                                                                                                                                       |
| `eboot_candidates` | string[] | yes      | List of executable filenames the boot path probes in order. `EBOOT.BIN` MUST precede `EBOOT.elf` if both are listed; the loader rejects the reverse order so a stale in-tree decrypt cannot shadow the canonical SCE-wrapped binary. |
| `year`             | integer  | yes      | Release year (`u16`).                                                                                                                                                                                                                |
| `developer`        | string   | yes      | Studio credit.                                                                                                                                                                                                                       |
| `engine`           | string   | yes      | Engine name (e.g. `"PhyreEngine"`, `"<studio> proprietary"`).                                                                                                                                                                        |
| `distribution`     | string   | yes      | One of `"psn-hdd"`, `"retail-hdd"`, `"disc-iso"`, `"firmware-exec"`, `"microtest"`. Lowercase kebab; the loader rejects other casings.                                                                                               |
| `rap_filename`     | string   | no       | NPDRM license file under the VFS `exdata/` dir; needed to decrypt PSN EBOOTs whose RAP name does not match the content id.                                                                                                            |
| `bench_max_steps`  | integer  | no       | Per-title instruction cap for `boot bench-once` and the title suites; defaults to 100,000,000. Raise it when a title's checkpoint sits past the default cap.                                                                          |
| `system_ver`       | string   | hdd/disc | The firmware the title's own `PARAM.SFO` asks for (`PS3_SYSTEM_VER`), as a store version key: `01.5000` is `"1.50"`. Required on every `hdd` / `disc` title, refused on `firmware-exec` and `manifest-relative` ones, which have no PARAM.SFO. Derives the reference cell; see `[[bench.matrix]]` below. A `title-corpus` suite holds it to the installed table. |

### `[checkpoint]` (required)

The deterministic stopping point. Both runners are expected to
reach the same checkpoint; mismatch is what the matrix's
Convergence column records.

| Field  | Type   | Required                | Notes                                                               |
| ------ | ------ | ----------------------- | ------------------------------------------------------------------- |
| `kind` | string | yes                     | `"process-exit"`, `"first-rsx-write"`, or `"pc"`.                   |
| `pc`   | string | only when `kind = "pc"` | Hex (`"0x10381ce8"`) or decimal. Hex requires the `0x`/`0X` prefix. |

Choosing a checkpoint:

- **`process-exit`** -- stops on `sys_process_exit`. Suitable
  for titles that exit deterministically on their own (for
  example, titles whose boot path probes uninitialized
  out-params and bails).
- **`first-rsx-write`** -- stops on the first PPU write into
  the RSX region (`0xC0000000+`). The earliest deterministic
  stopping point for titles whose attract loop never exits.
  Incompatible with `[rsx] mirror = true` (see below).
- **`pc = "0xADDR"`** -- stops when a step retires at a fixed
  guest PC. Used for targeted micro-investigations.

### `[source]` (optional)

| Field  | Type   | Required             | Notes                                                                                                                                                  |
| ------ | ------ | -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kind` | string | no                   | One of `"hdd"`, `"disc"`, `"firmware-exec"`, `"manifest-relative"`. Defaults to `"hdd"` when the block is absent.                                       |
| `path` | string | for the last two     | Directory holding the executable. Rejected on `hdd` / `disc`, which derive their directory from `content_id`.                                          |

`disc` titles are looked up under `vfs/dev_bdvd/`; `hdd` titles
under `vfs/dev_hdd0/game/`. The actual VFS root can be overridden
with the `CELLGOV_PS3_VFS_ROOT` env var.

The other two kinds ignore the VFS root entirely, because neither
is installed under a content-id directory. `firmware-exec` names
an executable shipped inside the firmware image and resolves
`path` against the process's current directory.
`manifest-relative` names one sitting beside the manifest -- the
microtests under `tests/micro/` -- and resolves `path` against
the manifest's own directory, so the reference means the same
thing from any working directory and needs no staging copy.

### `[rsx]` (optional)

| Field    | Type | Required | Notes                                                                                                                                                                                                                                                                                                                                                                                                                            |
| -------- | ---- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mirror` | bool | no       | When `true`, the RSX region (`0xC0000000+`) is mapped read-write so PPU stores at the GCM control-register window land in a guest-visible shadow and do not fault. Use this when the boot must run past RSX init. Defaults to `false`. **Incompatible with `checkpoint.kind = "first-rsx-write"`**; the mirror makes the RSX write succeed, so the checkpoint can never fire. The loader rejects this combination at parse time. |

### `[content]` (optional)

Read-only blobs the title's resource loader expects to find via
`sys_fs_open`. The boot-time content provider reads each host
file off disk and registers it in `Lv2Host::fs_store` at the
named `guest_path` before the step loop runs.

The block names no base directory of its own. A relative
`host_path` resolves against the directory the EBOOT sits in,
which for a PSN or disc install is the USRDIR holding the title's
data tree, unless the override env var points elsewhere. Neither
being available (the env var unset and the EBOOT path without a
parent directory) is a startup error, and so is any file missing
under the selected base; the error names the path it probed. The
repository carries no content of its own.

| Field               | Type                          | Required                 | Notes                                                                                                                                                          |
| ------------------- | ----------------------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `override_base_env` | string                        | no                       | Env-var name that, when set non-empty at run time, names the base directory in place of the EBOOT's own. Use it to point a boot at a stripped or modded tree. |
| `files`             | `{ guest_path, host_path }[]` | yes (when block present) | Each entry registers one blob. `host_path` is resolved against the selected base when relative; absolute paths pass through.                                  |

### `[[fs.mounts]]` (optional, array-of-tables)

Mount-table entries served by the cellFs VFS (`sys_fs_open` /
`sys_fs_stat` path miss + `sys_fs_opendir` snapshot
enumeration). Where `[content]` pre-registers explicit blobs,
mounts are the lazy disk-on-demand surface for the title's
resource enumerator.

| Field          | Type   | Required | Notes                                                                                                                                                                     |
| -------------- | ------ | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `prefix`       | string | yes      | Guest-side path prefix. MUST start with `/`. Each prefix must be unique within the manifest; duplicates are rejected.                                                     |
| `host`         | string | no       | Host-side directory the prefix maps to, POSIX-shaped, relative to the workspace root. Omitted, the prefix maps to the directory the EBOOT sits in.                        |
| `override_env` | string | no       | Env-var name whose non-empty value replaces `host` (or the EBOOT directory) at run time. Same role as `[content].override_base_env`.                                      |

### `[[bench.matrix]]` (optional, array-of-tables)

The **cells** this title declares beyond the one it derives. A cell
is the title at one firmware and one game version, and it is the
unit a result is keyed by: there is no "the result for `<disc
serial>`", only a result for `(<disc serial>, fw 4.91, base)`.

A title with a PARAM.SFO declares one cell without any row: its
`system_ver` times its base install. That is the **reference cell**,
the one the headline row of `docs/titles.md` renders, and it is
derived rather than declared so that nobody chooses it -- the title
was shipped and tested against that firmware, so a divergence there
is CellGov's and the syscall it names is one the title used on
hardware. Rows exist for exceptions:

- a further cell somebody wants measured (the newest firmware as a
  drift study, an update version),
- a per-cell `bench_max_steps` or `checkpoint` override, or a
  `pending` reason, attached to the derived cell by a row that
  repeats it,
- a `probe` at another cell.

A row that repeats the derived cell and carries none of those is
refused: it declares nothing the manifest has not. A row that repeats
it with `expect = "probe"` is refused too; the headline row states
whether the title converged.

A `firmware-exec` title has no PARAM.SFO, so its rows are its whole
declaration, one per firmware. A `manifest-relative` one likewise.

Three readers consume the declared set: `dev record-anchors` walks it,
`boot bench` gates the run against the selected cell's anchor under
that cell's cap and checkpoint, and `dev titles-gen` renders it.

Cells are declared here, never discovered from what a machine has
installed. A store holding forty firmwares does not give a title
forty cells.

| Field             | Type    | Required                    | Notes                                                                                     |
| ----------------- | ------- | --------------------------- | ------------------------------------------------------------------------------------------- |
| `fw`              | string  | yes                         | Firmware version key; the name of a `vfs/firmware/<key>/` entry.                          |
| `game_ver`        | string  | yes, except `firmware-exec` | `"base"` or an update version key. Refused on a `firmware-exec` title, whose version axis is the firmware's. |
| `expect`          | string  | no (default `"frontier"`)   | `"frontier"` or `"probe"`; see below.                                                     |
| `bench_max_steps` | integer | no                          | Per-cell override of the `[title]` cap.                                                   |
| `checkpoint`      | table   | no                          | Per-cell override of the `[checkpoint]` block; same `kind` / `pc` fields and refusals.    |
| `pending`         | string  | no                          | Why the cell carries no measurement yet; rendered beside the cell on the title's page.    |

`expect` says what the cell is for:

- **`frontier`** -- CellGov is expected to converge here, so a
  divergence names the next implementation target.
- **`probe`** -- the cell exists to observe an incompatibility.
  Which error the guest received is the datum; convergence is
  not. Running a 2012 title on 3.55 to see what it throws
  produces a divergence that is the desired result, and mixing
  that into the frontier map would make the "next target"
  reading false.

`pending` is what a declared cell says while it waits. The title's
page renders an unmeasured cell as `.`, and as `. (<reason>)` when the
row states one -- so a hole in the coverage grid can say why it is
there. The reason goes on a public page: state the defect or the
obstacle, not an issue number. An empty reason is refused, and so is
one carrying a `|` or a newline, neither of which survives the table
cell it renders in.

Per-cell overrides are whitelisted to `bench_max_steps` and
`checkpoint`, and no other key is accepted. Both are genuinely
properties of a cell: a different firmware library moves
steps-to-first-RSX-write, so a cap that makes a run reproducible
belongs to the cell, and a checkpoint reachable on one firmware
may be unreachable on another. Everything else stays title-level,
because two rows differing in a field the rendered document does
not show are two incomparable measurements presented as
comparable.

A title whose `[title]` states `system_ver = "2.76"` declares
`fw 2.76 x base` with no row. These two rows add a drift-study cell
and a probe beside it:

```toml
[[bench.matrix]]
fw = "4.91"
game_ver = "base"

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
bench_max_steps = 250_000_000
expect = "probe"
```

This row repeats the derived cell to raise its cap; without the
override it would be refused as adding nothing:

```toml
[[bench.matrix]]
fw = "2.76"
game_ver = "base"
bench_max_steps = 250_000_000
```

A `firmware-exec` title's matrix is one row per firmware, with no
`game_ver` at all:

```toml
[[bench.matrix]]
fw = "4.91"
```

## Worked examples

### Minimal PSN title (uses defaults)

```toml
[title]
content_id = "<PSN serial>"
short_name = "<short-name>"
display_name = "<Display Name>"
eboot_candidates = ["EBOOT.BIN", "EBOOT.elf"]
year = <release year>
developer = "<developer credit>"
engine = "<engine name>"
distribution = "psn-hdd"
system_ver = "<PS3_SYSTEM_VER as a version key, e.g. 1.50>"

[checkpoint]
kind = "first-rsx-write"
```

Defaults: `[source]` -> hdd, `[rsx] mirror` -> false, no
content / mounts, one declared cell (`fw <system_ver> x base`).

### Disc-ISO title

```toml
[title]
content_id = "<disc serial>"
...
distribution = "disc-iso"
system_ver = "<PS3_SYSTEM_VER as a version key, e.g. 2.76>"

[source]
kind = "disc"

[checkpoint]
kind = "first-rsx-write"
```

### Title that must run past RSX init

```toml
[title]
content_id = "<PSN serial>"
...

[checkpoint]
kind = "process-exit"

[rsx]
mirror = true

[[fs.mounts]]
prefix = "/app_home"
override_env = "CELLGOV_<CONTENT_ID>_CONTENT_DIR"
```

When `[rsx] mirror = true` is set, the checkpoint must be
something other than `first-rsx-write`; `process-exit` is the
usual choice for titles whose boot path probes for
unpopulated out-params and bails.

The `/app_home` mount, declaring no `host`, maps to the installed
title's own USRDIR, so every file the resource loader opens under
that prefix is served from the EBOOT's directory on demand. A
`[content]` block naming the same files would register the same
bytes ahead of the mount and change nothing the guest observes; it
is for a blob the guest opens at a path no mount serves.

## Adding a new title

1. Install the title with `cellgov title install <pkg> --rap <rap>`
   (PSN/HDD) or `cellgov title install <iso>` (a decrypted
   disc dump), which populates `vfs/dev_hdd0/game/<content_id>/USRDIR/`
   or `vfs/dev_bdvd/<content_id>/PS3_GAME/USRDIR/` respectively (both
   gitignored). `dev gen-manifest` can then emit a stub of this file,
   with `system_ver` read from the installed tree's `PARAM.SFO`.
2. Confirm `EBOOT.BIN` is present. CellGov decrypts it in
   memory via `cellgov_install::sce::decrypt_self_to_elf`; do
   NOT write the decrypted bytes back to `EBOOT.elf`; a stale
   on-disk copy can shadow the canonical SELF.
3. Write `title_manifests/<content_id>.toml` with the
   schema above.
4. Run `cellgov boot run --title <short_name>` once to
   confirm the boot path resolves the EBOOT.
5. Run `cellgov dev titles-gen` to refresh
   [../titles.md](../docs/titles.md) and the title's own page under
   `docs/titles/`. A title added to the registry without this step
   fails the drift gate on its missing page.

## Validation summary

The loader enforces, in addition to TOML well-formedness:

- All `[title]` and `[checkpoint]` fields populated, no
  unknown fields anywhere (`deny_unknown_fields`).
- `distribution` is one of the three accepted kebab tokens.
- `source.kind` is `"disc"` or `"hdd"` if present.
- `checkpoint.kind = "pc"` requires `pc = "..."`; the value
  parses as hex (with `0x` prefix) or decimal.
- `eboot_candidates` does not list `EBOOT.elf` before
  `EBOOT.BIN`.
- `[rsx] mirror = true` and `checkpoint.kind = "first-rsx-write"`
  are not combined.
- Every `[[fs.mounts]].prefix` starts with `/`; no two
  mounts share a prefix.
- `system_ver` is present on every `hdd` / `disc` title and absent
  from every `firmware-exec` / `manifest-relative` one. It is usable
  as a store directory name, like every `fw`.
- Every `fw`, and every `game_ver` other than `"base"`, is usable
  as a store directory name. This checks the key's shape only; the
  loader never consults the store, so a cell may name a firmware
  that is not installed.
- `game_ver` is present on every row of a title with a version
  axis, and absent from every row of a `firmware-exec` title.
- No two rows name the same cell. A row repeating the cell
  `system_ver` derives is accepted once, and only when it carries a
  `bench_max_steps`, `checkpoint` or `pending`; one carrying nothing
  is refused as adding nothing.
- A row repeating the derived cell is not a `probe`: the headline
  row states whether the title converged, and a probe cell's datum
  is the error the guest received instead.
- `pending` is neither empty nor carrying a `|` or a newline.

Any of these failures surfaces as a typed `ManifestError` at
startup, carrying the offending file path; a malformed
manifest is caught before the first guest step.
