# Title manifest guide

A title manifest is a single TOML file under
`docs/title_manifests/` that tells `cellgov_cli` how to boot one
PS3 title: where its EBOOT lives, what its boot harness should
treat as the stopping point, and which pieces of content the
boot path needs visible through the LV2 VFS.

A manifest describes the boot input for static-recompilation
analysis. It does not configure gameplay.

## Where they live and how they are discovered

- Path: `docs/title_manifests/<content_id>.toml`. The filename
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

The matrix rendered into [../titles.md](../titles.md) is
generated from this directory plus per-title fixture summaries
via `cellgov_cli titles-gen`.

## Schema overview

The file has one required `[title]` block, one required
`[checkpoint]` block, and four optional blocks (`[source]`,
`[rsx]`, `[content]`, `[[fs.mounts]]`). The TOML parser runs
with `deny_unknown_fields`; a typo in a key name surfaces as a
load error.

### `[title]` (required)

| Field              | Type     | Required | Notes                                                                                                                                                                                                                                |
| ------------------ | -------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `content_id`       | string   | yes      | PSN serial. Disc-ISO titles use `BCES.../BLES.../BLUS...`; PSN titles use `NPUA.../NPEA...`.                                                                                                                                         |
| `short_name`       | string   | yes      | Kebab-cased lookup label used by `--title`. Pick something short and stable; tests reference it.                                                                                                                                     |
| `display_name`     | string   | yes      | Human title for matrix output.                                                                                                                                                                                                       |
| `eboot_candidates` | string[] | yes      | List of executable filenames the boot path probes in order. `EBOOT.BIN` MUST precede `EBOOT.elf` if both are listed; the loader rejects the reverse order so a stale in-tree decrypt cannot shadow the canonical SCE-wrapped binary. |
| `year`             | integer  | yes      | Release year (`u16`).                                                                                                                                                                                                                |
| `developer`        | string   | yes      | Studio credit.                                                                                                                                                                                                                       |
| `engine`           | string   | yes      | Engine name (e.g. `"PhyreEngine"`, `"<studio> proprietary"`).                                                                                                                                                                        |
| `distribution`     | string   | yes      | One of `"psn-hdd"`, `"retail-hdd"`, `"disc-iso"`. Lowercase kebab; the loader rejects other casings.                                                                                                                                 |

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

| Field  | Type   | Required | Notes                                                                                                     |
| ------ | ------ | -------- | --------------------------------------------------------------------------------------------------------- |
| `kind` | string | no       | `"disc"` for disc-ISO titles, `"hdd"` for PSN/HDD installs. Defaults to `"hdd"` when the block is absent. |

`disc` titles are looked up under `tools/rpcs3/dev_bdvd/`; `hdd`
titles under `tools/rpcs3/dev_hdd0/game/`. The actual VFS root
can be overridden with the `CELLGOV_PS3_VFS_ROOT` env var.

### `[rsx]` (optional)

| Field    | Type | Required | Notes                                                                                                                                                                                                                                                                                                                                                                                                                            |
| -------- | ---- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mirror` | bool | no       | When `true`, the RSX region (`0xC0000000+`) is mapped read-write so PPU stores at the GCM control-register window land in a guest-visible shadow and do not fault. Use this when the boot must run past RSX init. Defaults to `false`. **Incompatible with `checkpoint.kind = "first-rsx-write"`**; the mirror makes the RSX write succeed, so the checkpoint can never fire. The loader rejects this combination at parse time. |

### `[content]` (optional)

Read-only blobs the title's resource loader expects to find via
`sys_fs_open`. The boot-time content provider reads each host
file off disk and registers it in `Lv2Host::fs_store` at the
named `guest_path` before the step loop runs.

| Field               | Type                          | Required                 | Notes                                                                                                                                                                                         |
| ------------------- | ----------------------------- | ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `base`              | string                        | yes (when block present) | Resolved against the workspace root when relative.                                                                                                                                            |
| `override_base_env` | string                        | no                       | Env-var name that, when set at run time, replaces `base`. Use it to point at a gitignored local copy of the real title content while a synthetic public copy stays checked in as the default. |
| `files`             | `{ guest_path, host_path }[]` | yes (when block present) | Each entry registers one blob. `host_path` is resolved against `base` when relative; absolute paths pass through.                                                                             |

### `[[fs.mounts]]` (optional, array-of-tables)

Mount-table entries served by the cellFs VFS (`sys_fs_open` /
`sys_fs_stat` path miss + `sys_fs_opendir` snapshot
enumeration). Where `[content]` pre-registers explicit blobs,
mounts are the lazy disk-on-demand surface for the title's
resource enumerator.

| Field          | Type   | Required | Notes                                                                                                                 |
| -------------- | ------ | -------- | --------------------------------------------------------------------------------------------------------------------- |
| `prefix`       | string | yes      | Guest-side path prefix. MUST start with `/`. Each prefix must be unique within the manifest; duplicates are rejected. |
| `host`         | string | yes      | Host-side directory the prefix maps to.                                                                               |
| `override_env` | string | no       | Env-var name that replaces `host` at run time. Same role as `[content].override_base_env`.                            |

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

[checkpoint]
kind = "first-rsx-write"
```

Defaults: `[source]` -> hdd, `[rsx] mirror` -> false, no
content / mounts.

### Disc-ISO title

```toml
[title]
content_id = "<disc serial>"
...
distribution = "disc-iso"

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

[content]
base = "boot_content/<content_id>"
override_base_env = "CELLGOV_<CONTENT_ID>_CONTENT_DIR"
files = [
    { guest_path = "/app_home/Data/Resources/first.xml",  host_path = "Data/Resources/first.xml" },
    { guest_path = "/app_home/Data/Local/Localization.xml", host_path = "Data/Local/Localization.xml" },
]

[[fs.mounts]]
prefix = "/app_home"
host = "boot_content/<content_id>"
override_env = "CELLGOV_<CONTENT_ID>_CONTENT_DIR"
```

When `[rsx] mirror = true` is set, the checkpoint must be
something other than `first-rsx-write`; `process-exit` is the
usual choice for titles whose boot path probes for
unpopulated out-params and bails.

## Adding a new title

1. Find or create the title's USRDIR under `tools/rpcs3/dev_hdd0/game/<content_id>/USRDIR/` (PSN/HDD) or
   `tools/rpcs3/dev_bdvd/<content_id>/PS3_GAME/USRDIR/` (disc).
   Both directories are gitignored.
2. Confirm `EBOOT.BIN` is present. CellGov decrypts it in
   memory via `cellgov_firmware::sce::decrypt_self_to_elf`; do
   NOT write the decrypted bytes back to `EBOOT.elf`; a stale
   on-disk copy can shadow the canonical SELF.
3. Write `docs/title_manifests/<content_id>.toml` with the
   schema above.
4. Run `cellgov_cli run-game --title <short_name>` once to
   confirm the boot path resolves the EBOOT.
5. If a cross-runner fixture exists or will be captured, run
   `cellgov_cli titles-gen` to refresh
   [../titles.md](../titles.md).

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

Any of these failures surfaces as a typed `ManifestError` at
startup, carrying the offending file path; a malformed
manifest is caught before the first guest step.
