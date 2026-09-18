# Firmware PUP corpus

> **Never put PUP bytes in this repository.** Keep every PUP in an
> operator-owned directory outside the clone. This rule also applies to
> ignored directories inside the clone: an ignore rule is not permission
> to store firmware there.

This runbook starts with PUP files the operator already holds. It does not
describe where or how to obtain them. It records those files in the LV2
archive and checks the files against that record.

## Directory layout

Keep the corpus and CellGov's writable state outside the repository. One
possible layout is:

```text
<operator-data>/
  ps3-pups/
    <any subdirectories>/
      <descriptive-name>.PUP
  cellgov-state/
    dev_hdd0/
```

`firmware verify-corpus` takes the `ps3-pups` directory as its positional
`DIR` argument. It walks subdirectories and reads regular files whose
extension is `.PUP`, without using the file name as identity. The PUP bytes
provide the hash, firmware version, image version, and size.

The state directory is separate. Commands point at it with:

```text
--vfs-root <operator-data>/cellgov-state/dev_hdd0
```

Do not set `CELLGOV_DUMPS_DIR` for this workflow. That variable selects
test-corpus locations; it does not configure `firmware verify-corpus`.

## Build and vault

Build the CLI with decryption enabled:

```console
cargo build --release -p cellgov_cli --features decrypt
```

The commands below spell the resulting binary as `cellgov`. Use
`target/release/cellgov.exe` on Windows or `target/release/cellgov` on
Linux when it is not on `PATH`.

CellGov contains no keys. Give decrypting commands an operator-owned vault
in one of these ways:

```text
CELLGOV_KEYS=<key-file-or-directory>
```

or normalize the vault into the same state root used by the commands:

```console
cellgov keys import <key-file-or-directory> --output <operator-data>/cellgov-state
```

With the default VFS root, the normalized vault is
`vfs/.cellgov/keys/keys.toml`. This runbook instead keeps the equivalent
file outside the clone by selecting an external VFS root.

The normalized file is
`<operator-data>/cellgov-state/.cellgov/keys/keys.toml`; it is the vault
read by commands that use
`--vfs-root <operator-data>/cellgov-state/dev_hdd0`.
When measuring in a different disposable state root, point
`CELLGOV_KEYS` at that normalized file for the duration of the command.

Scanning a PUP does not require decryption. However, `verify-corpus` also
checks any installed firmware whose recorded PUP hash names an archive row.
That check opens the stored modules. If the vault lacks a required key, the
command stops instead of calling the installed tree mismatched. A PUP that
is absent from the corpus and a PUP that is present while the vault cannot
open an installed tree are different states.

## Register one PUP

`docs/lv2/pup.tsv` is a hand-curated table. One row represents one exact PUP
image, keyed by the SHA-256 over its file bytes. Two PUPs that declare the
same firmware version still need separate rows when their hashes differ.

Use a disposable state root while measuring a PUP. This prevents a second
image for the same firmware version from replacing the first before its row
is recorded.

1. Install the PUP and note the firmware version printed by the command:

   ```console
   cellgov --vfs-root <scratch-state>/dev_hdd0 firmware install <corpus-file>.PUP
   ```

2. Read the installed identity as JSON:

   ```console
   cellgov --vfs-root <scratch-state>/dev_hdd0 --format json firmware show <version>
   ```

   Copy `version`, `pup_sha256`, and `image_version` from the single
   `firmware` entry. The PUP hash here is over the original PUP file, not an
   extracted module.

3. Record the original file length in bytes. On PowerShell:

   ```powershell
   (Get-Item -LiteralPath '<corpus-file>.PUP').Length
   ```

   On Linux:

   ```console
   stat -c %s -- '<corpus-file>.PUP'
   ```

4. Look for `pup_sha256` in the table's first column. If it already has a
   row, compare every cell with the new measurement and do not add a second
   row. Treat any disagreement as a finding. If the hash is absent, add one
   tab-separated row in the existing column order:

   ```text
   <pup_sha256><TAB><fw><TAB><size_bytes><TAB><image_version><TAB><source_note><TAB><acquired>
   ```

   Use tab characters between cells. Keep rows sorted by `pup_sha256`.
   `source_note` is a general provenance label, not a path, URL, or
   acquisition instruction. Use the recorded acquisition date as
   `YYYY-MM-DD`, or `none` when no date was recorded. No cell is empty.

5. Remove the disposable state or choose another empty state root before
   measuring the next PUP. Never use `--force` until the current row has
   been recorded.

## Validate the table

Run the archive gate after every edit:

```console
cargo test -p cellgov_lv2 --test lv2_archive pup_rows_are_well_formed
cargo test -p cellgov_lv2 --test lv2_archive committed_tables_load_and_reference_each_other
```

These tests check the row shapes, key order, uniqueness, and firmware
references. They do not prove that the operator still holds the recorded
bytes.

The CLI compiles `pup.tsv` into the binary, so rebuild it after the table
changes:

```console
cargo build --release -p cellgov_cli --features decrypt
```

Then point the rebuilt corpus verifier at the operator-owned directory:

```console
cellgov --vfs-root <operator-data>/cellgov-state/dev_hdd0 --format json firmware verify-corpus <operator-data>/ps3-pups
```

Read the JSON categories separately:

- `present` contains archive rows whose file bytes and PUP metadata match;
- `missing` contains archive rows for which the directory has no matching
  file;
- `mismatched` contains corpus files that disagree with the archive;
- `installed` contains the existing firmware-verification reports for
  installed entries linked to archive rows.

The pass is clean only when every archive row is present, no corpus file or
installed identity mismatches, and every linked installed entry verifies.
A partial corpus is valid input, but it is not a clean pass: leave its absent
rows in `missing` rather than deleting archive rows to make the result green.

The [generated CLI reference](cli.md#cellgov-firmware-verify-corpus) owns the
exact options and exit statuses. The [LV2 archive page](lv2/README.md#pup-provenance)
owns the current table measurements and column definitions; do not copy
those changing values into this runbook.
