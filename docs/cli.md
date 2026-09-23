# cellgov command reference

`cellgov` is the one binary: it installs a PS3 game, boots it through
the deterministic runtime, and diffs the result. This page is the full
reference. `cellgov --help` and `cellgov <command> --help` print the
same descriptions, examples, and flags in terminal form.

Every command accepts `-h`/`--help`, and `cellgov --version` reports the
build. Flags take either `--flag value` or `--flag=value`. A scalar flag
repeated at one level is refused rather than taking the last value; a
global repeated on both sides of the command name is the one exception,
where the spelling nearer the command wins.

## Exit codes

```
Exit codes:
  0    success
  1    the operation ran and failed
  2    usage error
  3    runs that had to reproduce each other disagreed
  4    a subprocess failed, or a verification diverged
  5    a boot moved off its committed anchor
  >=10 an outcome particular to one command; its own help names it
```

A command with an outcome the contract above does not cover gives it a
status of 10 or more and names it in its own section below.

## Global options

Accepted anywhere on the line, before or after the command. A global
that the named command would ignore is a usage error rather than a
silent no-op, so a flag that reached the wrong command is visible.

| Option | Value | Description |
| --- | --- | --- |
| `--vfs-root` | `DIR` | PS3 VFS root (default: CELLGOV_PS3_VFS_ROOT, then vfs/dev_hdd0). |
| `--format` | `FORMAT` | Report rendering. One of `human`, `json`. Default `human`. |
| `--quiet` | -- | Suppress progress and other non-essential stderr. |
| `-v, --verbose` | -- | Print more detail about what the command did. |
| `--no-color` | -- | Never emit SGR colour sequences. |
| `--no-progress` | -- | Never render a progress bar. |
| `--force-ansi` | -- | Assume a Windows console with no VT marker processes ANSI sequences. |
| `--no-input` | -- | Never prompt; a needed confirmation becomes a usage error. |
| `-y, --yes` | -- | Answer every confirmation prompt yes. |

## Streams

Results go to stdout: tables, JSON, requested data, and nothing else.
Everything about producing the result goes to stderr: the progress bar,
the selection banner, warnings, and errors. A `--format json` consumer
can therefore read stdout blind.

## Terminal behavior

Three properties of the progress display hold for every command that
renders one.

- **Escape sequences on Windows.** A console processes them only when
  its terminal turned VT on, and nothing reports which one did, so the
  bar animates when `WT_SESSION`, `ConEmuANSI=ON`, `ANSICON`,
  `TERM_PROGRAM` or `TERM` names a terminal that does, and falls back to
  plain threshold lines otherwise. `--force-ansi`, or `CELLGOV_FORCE_ANSI`
  set once for the console, answers for a host that exports none of them.
- **Interruption.** A Ctrl-C while a bar is up stops the render
  thread, restores the cursor and clears the taskbar state, then ends
  the process as the default action would: killed by SIGINT on Unix,
  so a shell loop driving the command breaks; exited
  `STATUS_CONTROL_C_EXIT` on Windows. A command whose child boot died
  that way dies the same way. A `kill -9` or a crash skips the restore;
  `tput cnorm`, or any command that resets the terminal, brings the
  cursor back. The interruption itself costs nothing: an install writes
  into a `.staging-*` sibling and commits by rename, so an interrupted
  run leaves the previous tree intact and the next run sweeps the
  residue.
- **The render thread is presentation-only.** Worker threads never
  block on it, so a wedged or redirected stderr cannot stall an install
  or a bench run.

A command that streams lines while it works caps the bar at plain
threshold lines, which interleave harmlessly; the animated bar is
reserved for commands that are quiet until they report.

## Environment

| Variable | Scope | Effect |
| --- | --- | --- |
| `CELLGOV_KEYS` | operator | Override the key-vault file. |
| `CELLGOV_PS3_VFS_ROOT` | operator | Override the PS3 VFS root. |
| `CELLGOV_<TITLE_ID>_CONTENT_DIR` | operator | Override one title's installed content directory. |
| `CELLGOV_NO_COLOR` | operator | Disable color for this program. |
| `CELLGOV_FORCE_ANSI` | operator | Force ANSI terminal output. |
| `CELLGOV_FW_DEBUG` | debug | Trace firmware package decryption. |
| `CELLGOV_BOOT_TRACE_MEM` | debug | Record boot memory tracing. |
| `CELLGOV_RUNGAME_PROFILE` | debug | Print host-time boot spans. |
| `CELLGOV_HLE_RETURN_WATCH` | debug | Watch HLE return NIDs. |
| `CELLGOV_HLE_RETURN_WATCH_PCS` | debug | Limit an HLE watch to PCs. |
| `CELLGOV_HLE_RETURN_WATCH_PATH` | debug | Write HLE watch records to a file. |
| `CELLGOV_STORE_WATCH` | debug | Watch guest stores in an address range. |
| `CELLGOV_STORE_WATCH_PATH` | debug | Write store-watch records to a file. |
| `CELLGOV_VALUE_SAMPLE` | debug | Sample guest values in an address range. |
| `CELLGOV_VALUE_SAMPLE_PATH` | debug | Write value samples to a file. |
| `CELLGOV_VALUE_SAMPLE_STRIDE` | debug | Set the value-sample stride. |
| `CELLGOV_NO_FIRMWARE_DIR` | test-only | Suppress the synthetic firmware-directory default. |
| `CELLGOV_OBS_NULL_SINK` | test-only | Discard observation output in a synthetic run. |
| `CELLGOV_RETAIN_SCRATCH` | test-only | Retain a scratch directory after a test. |


## JSON output

`--format json` writes one document to stdout, with no ANSI and no
other output on that stream.

The store's read commands publish the versioned documents below. Their
field names are stable and each document carries `format_version`,
bumped only for a change a reader of the previous version could not
survive -- a new field is additive and leaves it alone.

`diff compare`, `diff observations` and `explore` also accept
`--format json`. Those documents are the runtime's own observation and
comparison records rather than a published API: they carry no
`format_version` and are not reproduced here.

`status`:

```json
{
  "format_version": 2,
  "store": "vfs",
  "store_bytes": 21474836480,
  "unreadable_paths": 0,
  "firmware": [
    {
      "version": "4.93",
      "entry_dir": "firmware/4.93",
      "record": ".cellgov/installs/firmware/4.93.install.toml",
      "pup_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "image_version": "0x0004009300000000",
      "modules": 370,
      "core_os": {
        "kernel": {
          "path": "core_os/lv2_kernel.self",
          "stored_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "files": [
          {
            "name": "lv1.self",
            "size": 1280160
          },
          {
            "name": "lv2_kernel.self",
            "size": 1586440
          }
        ]
      }
    }
  ],
  "titles": [
    {
      "title_id": "NPUA80001",
      "short_name": "flow",
      "display_name": "flOw",
      "base": {
        "version": "01.00",
        "version_key": "app_ver",
        "dir": "dev_hdd0/game/NPUA80001",
        "tree": "game",
        "distribution": "psn-hdd",
        "source_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
        "system_ver": "01.5000",
        "record": ".cellgov/installs/titles/NPUA80001/base.install.toml"
      },
      "ships_in_firmware": false,
      "updates": [
        {
          "version": "1.02",
          "version_key": "app_ver",
          "dir": "titles/NPUA80001/updates/1.02/game",
          "source_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "min_system_ver": "03.5500",
          "system_ver": "03.5500",
          "record": ".cellgov/installs/titles/NPUA80001/update-1.02.install.toml"
        }
      ],
      "anchors": [
        {
          "fw": "1.50",
          "game_ver": "base",
          "expect": "frontier",
          "reference": true,
          "recorded": true,
          "installed": true
        },
        {
          "fw": "4.93",
          "game_ver": "base",
          "expect": "frontier",
          "reference": false,
          "recorded": true,
          "installed": true
        }
      ]
    }
  ]
}
```

`firmware list`, `firmware show`:

```json
{
  "format_version": 2,
  "store": "vfs",
  "firmware": [
    {
      "version": "4.93",
      "entry_dir": "firmware/4.93",
      "record": ".cellgov/installs/firmware/4.93.install.toml",
      "pup_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "image_version": "0x0004009300000000",
      "modules": 370,
      "core_os": {
        "kernel": {
          "path": "core_os/lv2_kernel.self",
          "stored_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "files": [
          {
            "name": "lv1.self",
            "size": 1280160
          },
          {
            "name": "lv2_kernel.self",
            "size": 1586440
          }
        ]
      }
    }
  ]
}
```

`title list`, `title show`:

```json
{
  "format_version": 2,
  "store": "vfs",
  "titles": [
    {
      "title_id": "NPUA80001",
      "short_name": "flow",
      "display_name": "flOw",
      "base": {
        "version": "01.00",
        "version_key": "app_ver",
        "dir": "dev_hdd0/game/NPUA80001",
        "tree": "game",
        "distribution": "psn-hdd",
        "source_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
        "system_ver": "01.5000",
        "record": ".cellgov/installs/titles/NPUA80001/base.install.toml"
      },
      "ships_in_firmware": false,
      "updates": [
        {
          "version": "1.02",
          "version_key": "app_ver",
          "dir": "titles/NPUA80001/updates/1.02/game",
          "source_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "min_system_ver": "03.5500",
          "system_ver": "03.5500",
          "record": ".cellgov/installs/titles/NPUA80001/update-1.02.install.toml"
        }
      ],
      "anchors": [
        {
          "fw": "1.50",
          "game_ver": "base",
          "expect": "frontier",
          "reference": true,
          "recorded": true,
          "installed": true
        },
        {
          "fw": "4.93",
          "game_ver": "base",
          "expect": "frontier",
          "reference": false,
          "recorded": true,
          "installed": true
        }
      ]
    }
  ]
}
```

`firmware verify`, `title verify`:

```json
{
  "format_version": 2,
  "store": "vfs",
  "subject": "NPUA80001",
  "entries": [
    {
      "entry": "base",
      "matched": 128,
      "divergences": [
        {
          "path": "dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN",
          "kind": "modified",
          "expected": "0000000000000000000000000000000000000000000000000000000000000000",
          "found": "0000000000000000000000000000000000000000000000000000000000000000"
        }
      ]
    }
  ],
  "clean": false
}
```

`firmware verify-pups`:

```json
{
  "format_version": 2,
  "pup_directory": "dumps/firmware",
  "present": [
    {
      "fw": "4.93",
      "pup_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "size_bytes": 206197916,
      "image_version": "0x0000000000010b94",
      "path": "PS3UPDAT-4.93.PUP"
    }
  ],
  "missing": [
    {
      "fw": "1.94",
      "pup_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
      "size_bytes": 125289664,
      "image_version": "0x0000000000001d56"
    }
  ],
  "mismatched": [
    {
      "subject": "PS3UPDAT-4.92.PUP",
      "kind": "sha256",
      "fw": "4.92",
      "expected": [
        "2222222222222222222222222222222222222222222222222222222222222222"
      ],
      "found": "3333333333333333333333333333333333333333333333333333333333333333"
    },
    {
      "subject": "damaged.PUP",
      "kind": "invalid-pup",
      "expected": [],
      "found": "4444444444444444444444444444444444444444444444444444444444444444",
      "reason": "PUP header is truncated"
    }
  ],
  "installed": [
    {
      "entry": "4.93",
      "matched": 370,
      "divergences": []
    }
  ],
  "clean": false
}
```

`firmware kernels`:

```json
{
  "format_version": 2,
  "store": "vfs",
  "vault": "vfs/.cellgov/keys/keys.toml",
  "entries": [
    {
      "version": "1.50",
      "state": "no_key",
      "detail": "an LV2 keyset for firmware 1.50 (the vault holds none)",
      "kernel_version": "1.50"
    },
    {
      "version": "2.76",
      "state": "not_unpacked",
      "detail": "update_files carries no CORE_OS_PACKAGE.pkg"
    },
    {
      "version": "3.60",
      "state": "not_installed"
    },
    {
      "version": "4.93",
      "state": "decrypted",
      "kernel_version": "4.93",
      "elf_bytes": 3145728,
      "elf_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
    }
  ]
}
```

## `cellgov`

Deterministic PS3 oracle: install a PS3 game, boot it, and diff the result.

```console
$ cellgov title install dumps/NPUA80001/flow.pkg --rap dumps/NPUA80001/flow.rap
$ cellgov boot bench --title flow --fw 4.93
$ cellgov diff observations cellgov.json rpcs3.json
```

```
Usage: cellgov [OPTIONS] <COMMAND>
```

### `cellgov status`

What this machine holds, and which cells have an anchor.

```console
$ cellgov status
$ cellgov status --format json
```

```
Usage: cellgov status [OPTIONS]
```

### `cellgov firmware`

Installed PS3 system software.

```
Usage: cellgov firmware [OPTIONS] <COMMAND>
```

#### `cellgov firmware install`

Install system software from a PS3UPDAT.PUP.

```console
$ cellgov firmware install dumps/firmware/PS3UPDAT.PUP
$ cellgov firmware install dumps/firmware/PS3UPDAT.PUP --force --verbose
$ cellgov firmware install dumps/firmware/PS3UPDAT.PUP --kernel-only
```

```
Usage: cellgov firmware install [OPTIONS] <PATH>
```

| Argument | Description |
| --- | --- |
| `PATH` | The PS3UPDAT.PUP to install. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--force` | -- | Replace the version that is already installed. |
| `--kernel-only` | -- | Add only the LV2 kernel to the entry this PUP already installed; its dev_flash tree stays as it is. |
| `--output` | `DIR` | Store root (default: the directory enclosing the PS3 VFS root). |

#### `cellgov firmware list`

Name every installed firmware version.

```console
$ cellgov firmware list
$ cellgov firmware list --format json
```

```
Usage: cellgov firmware list [OPTIONS]
```

#### `cellgov firmware show`

Report one installed version in full.

```console
$ cellgov firmware show 4.93
```

```
Usage: cellgov firmware show [OPTIONS] <VERSION>
```

| Argument | Description |
| --- | --- |
| `VERSION` | Version key of a `firmware/<key>/` entry. Required. |

#### `cellgov firmware verify`

Re-hash an installed version against its manifest and record.

```console
$ cellgov firmware verify 4.93
$ cellgov firmware verify 4.93 --format json
```

```
Usage: cellgov firmware verify [OPTIONS] <VERSION>
```

| Argument | Description |
| --- | --- |
| `VERSION` | Version key of a `firmware/<key>/` entry. Required. |

```
Exit codes:
  0   every recorded artefact matched
  4   the tree diverged from what its record holds
```

#### `cellgov firmware verify-pups`

Verify a directory of PUP files against the LV2 archive.

```console
$ cellgov firmware verify-pups dumps/firmware
```

```
Usage: cellgov firmware verify-pups [OPTIONS] <DIR>
```

| Argument | Description |
| --- | --- |
| `DIR` | Directory that holds the PUP files. Required. |

```
Exit codes:
  0   every archive PUP and every installed tree whose PUP hash names an archive row matched
  4   a PUP was missing or mismatched, or one of those installed trees diverged
```

#### `cellgov firmware kernels`

Refresh the operator-local LV2 kernel coverage report under `<store>/.cellgov/firmware-kernel-coverage.json`.

```console
$ cellgov firmware kernels
$ cellgov firmware kernels --format json
```

```
Usage: cellgov firmware kernels [OPTIONS]
```

```
Exit codes particular to this command:
  41  a stored kernel yielded no ELF for a reason other than a missing key
```

#### `cellgov firmware uninstall`

Remove an installed firmware version.

```console
$ cellgov firmware uninstall 4.93 --dry-run
$ cellgov firmware uninstall 4.93 --verify
```

```
Usage: cellgov firmware uninstall [OPTIONS] <VERSION>
```

| Argument | Description |
| --- | --- |
| `VERSION` | Version key of a `firmware/<key>/` entry. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--verify` | -- | Re-hash the installed tree against its manifest and record first. |
| `--force` | -- | Remove it even though a committed anchor names it, or though `--verify` found the tree diverged. |
| `--dry-run` | -- | Print the removal plan and stop. |

### `cellgov title`

Installed games and their updates.

```
Usage: cellgov title [OPTIONS] <COMMAND>
```

#### `cellgov title install`

Install a base title from a PKG or a decrypted disc image.

```console
$ cellgov title install dumps/NPUA80001/flow.pkg --rap dumps/NPUA80001/flow.rap
$ cellgov title install dumps/BCES00664/wipeout.iso
```

```
Usage: cellgov title install [OPTIONS] <PKG|ISO>
```

| Argument | Description |
| --- | --- |
| `PKG\|ISO` | A PKG or a decrypted ISO; the container kind is read from the file. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--rap` | `PATH` | RAP for a license-1/2 NPDRM title. |
| `--force` | -- | Replace an existing install of this title. |
| `--no-firmware` | -- | Leave the system software a disc image ships uninstalled and unrecorded, instead of registering it as the firmware entry the title was certified against. |
| `--output` | `DIR` | Store root (default: the directory enclosing the PS3 VFS root). |

#### `cellgov title install-update`

Install a GD/HG update PKG over an installed base.

```console
$ cellgov title install-update dumps/NPUA80068/sshd-1.02.pkg
```

```
Usage: cellgov title install-update [OPTIONS] <PATH>
```

| Argument | Description |
| --- | --- |
| `PATH` | The container to install. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--force` | -- | Replace what is already installed there. |
| `--output` | `DIR` | Store root (default: the directory enclosing the PS3 VFS root). |

#### `cellgov title list`

Name every installed title, its base, and its updates.

```console
$ cellgov title list
$ cellgov title list --format json
```

```
Usage: cellgov title list [OPTIONS]
```

#### `cellgov title show`

Report one installed title in full.

```console
$ cellgov title show NPUA80001
```

```
Usage: cellgov title show [OPTIONS] <TITLE_ID>
```

| Argument | Description |
| --- | --- |
| `TITLE_ID` | Title id of a store entry. Required. |

#### `cellgov title verify`

Re-hash an installed title against its records.

```console
$ cellgov title verify NPUA80001
$ cellgov title verify NPUA80068 --ver 1.02
```

```
Usage: cellgov title verify [OPTIONS] <TITLE_ID>
```

| Argument | Description |
| --- | --- |
| `TITLE_ID` | Title id of a store entry. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--ver` | `V` | Check this version alone: `base`, or an update version key. |

```
Exit codes:
  0   every recorded artefact matched
  4   the tree diverged from what its record holds
```

#### `cellgov title uninstall`

Remove an installed title, or one of its versions.

```console
$ cellgov title uninstall NPUA80001 --all --dry-run
$ cellgov title uninstall NPUA80068 --ver 1.02
```

```
Usage: cellgov title uninstall [OPTIONS] <TITLE_ID>
```

| Argument | Description |
| --- | --- |
| `TITLE_ID` | Title id to remove. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--ver` | `V` | Remove one version alone: `base`, or an update version key. |
| `--updates` | -- | Remove every installed update, keeping the base. |
| `--all` | -- | Remove every update and the base. |
| `--verify` | -- | Re-hash the live tree against the install record first. |
| `--keep-rap` | -- | Leave the RAP in exdata; another title may share it. |
| `--force` | -- | Remove even when `--verify` finds a modified tree. |
| `--dry-run` | -- | Print the removal plan and stop. |
| `--output` | `DIR` | Store root (default: the directory enclosing the PS3 VFS root). |

### `cellgov keys`

The operator's key vault.

```
Usage: cellgov keys [OPTIONS] <COMMAND>
```

#### `cellgov keys show`

Inventory of the vault a decrypt would read.

```console
$ cellgov keys show
```

```
Usage: cellgov keys show [OPTIONS] [PATH]
```

| Argument | Description |
| --- | --- |
| `PATH` | Read this vault instead of the configured one. |

| Option | Value | Description |
| --- | --- | --- |
| `--output` | `DIR` | Store root (default: the directory enclosing the PS3 VFS root). |

```
Exit codes particular to this command:
  40  the vault loaded, but a decrypt path would find a key missing
```

#### `cellgov keys import`

Normalize a keys file or directory into the store's vault.

```console
$ cellgov keys import dumps/keys/
$ cellgov keys import dumps/keys/keys.toml --replace
```

```
Usage: cellgov keys import [OPTIONS] <PATH>
```

| Argument | Description |
| --- | --- |
| `PATH` | Keys file or directory to read. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--replace` | -- | Drop the vault already installed instead of merging into it. |
| `--output` | `DIR` | Store root (default: the directory enclosing the PS3 VFS root). |

#### `cellgov keys remove`

Delete the store's vault.

```console
$ cellgov keys remove
```

```
Usage: cellgov keys remove [OPTIONS]
```

| Option | Value | Description |
| --- | --- | --- |
| `--output` | `DIR` | Store root (default: the directory enclosing the PS3 VFS root). |

### `cellgov self`

Operations on a SELF outside the store.

```
Usage: cellgov self [OPTIONS] <COMMAND>
```

#### `cellgov self decrypt`

Write a SELF's plaintext ELF to a file.

```console
$ cellgov self decrypt vfs/dev_flash/sys/external/liblv2.sprx --output liblv2.elf
```

```
Usage: cellgov self decrypt [OPTIONS] <SELF>
```

| Argument | Description |
| --- | --- |
| `SELF` | The SELF to decrypt. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--output` | `PATH` | Where to write the plaintext ELF (default: alongside the input). |
| `--rap` | `PATH` | Use this RAP instead of the one under the VFS root. |

```
SCE-wrapped input:
  this build has no decrypt support: plaintext ELF / PRX only. An
  SCE-wrapped input is refused by name, and --vfs-root names no path
  this build reads; rebuild with --features decrypt to read one.
```

### `cellgov boot`

Boot a title through the deterministic runtime.

```
Usage: cellgov boot [OPTIONS] <COMMAND>
```

```
Firmware selection:
  --fw names an installed version and outranks the record. Without it,
  a disc title boots the firmware its install record says it shipped
  with. When the store does not hold that version, it refuses by name.
  Any other title, and a disc whose record names none, boots the only
  installed firmware. With none or several installed, the store
  refuses. A disc record names none when:
    - the disc carried no update package;
    - the install declined it;
    - the record is older than the field.
  Nothing prompts. --firmware-dir names a tree outside the store and
  marks the run unmanaged.
```

#### `cellgov boot run`

Boot a title and report where it stopped.

```console
$ cellgov boot run --title flow --fw 4.93
$ cellgov boot run --title sshd --fw 4.93 --max-steps 200000 --prescan
$ cellgov boot run --title wipeout --fw 4.93 --trace --save-state-trace wipeout.state
```

```
Usage: cellgov boot run [OPTIONS] <--title <NAME>|--content-id <ID>|--title-manifest <PATH>> [ELF]
```

| Argument | Description |
| --- | --- |
| `ELF` | Boot this executable instead of the one the composition resolves. |

| Option | Value | Description |
| --- | --- | --- |
| `--title` | `NAME` | Short name from the title registry. |
| `--content-id` | `ID` | Content id (serial) from the title registry. |
| `--title-manifest` | `PATH` | A title manifest outside the registry. |
| `--fw` | `VERSION` | Installed firmware version. Without it, a disc title boots the firmware its record says it shipped with. Any other title, or a disc whose record names none, boots the only installed firmware. With none or several installed, the store refuses. |
| `--game-ver` | `base\|VERSION` | Installed content version; may be omitted when exactly one is a candidate. |
| `--firmware-dir` | `DIR` | A `sys/external` tree outside the store. Marks the run unmanaged, so it carries no firmware version. |
| `--max-steps` | `N` | Retire at most this many steps. Default `100000`. |
| `--budget` | `N` | Simulated-time budget for the run. |
| `--trace` | -- | Emit the binary trace stream. |
| `--profile` | -- | Report per-opcode execution counts. |
| `--profile-pairs` | -- | Report the hottest consecutive opcode pairs. |
| `--strict-reserved` | -- | Fault on a read of a reserved region instead of serving zeroes. |
| `--prescan` | -- | Scan the title for unimplemented opcodes before booting. |
| `--dump-at-pc` | `HEX` | Dump PPU state each time this guest PC retires. |
| `--dump-skip` | `N` | Skip this many `--dump-at-pc` hits before dumping. Default `0`. |
| `--dump-mem-boot` | `HEX[,HEX]` | Guest addresses to dump once the image is loaded. |
| `--dump-mem-fault` | `HEX[:LEN][,...]` | Guest ranges to dump if the run faults. |
| `--patch-byte` | `ADDR=VALUE[,...]` | Bytes to overwrite in the loaded image before the first step. |
| `--save-observation` | `PATH` | Write the run's observation JSON here. |
| `--observation-manifest` | `PATH` | Checkpoint manifest naming the regions the observation covers. |
| `--save-boot-summary` | `PATH` | Write the run's boot summary JSON here. |
| `--save-state-trace` | `PATH` | Write the run's state trace here. |
| `--guest-arg` | `VALUE` | One guest argv entry; repeat for more. Values may spell a flag. |
| `--skip-module-start` | -- | Run no firmware module's module_start in the boot process. |
| `--force-system-authid` | -- | Serve the system-class bdj.self program authority id instead of the one the title's SELF names. |
| `--prx-base` | `HEX` | Load the firmware module set at this 64K-aligned base inside the main region, instead of the first 64K page past the title image. A spawned child's firmware set loads at the same base. |
| `--disable-module-start-hle-stubs` | -- | Run the LLE path of each module_start the boot stubs to CELL_OK. |

```
Exit codes particular to this command:
  10  the guest faulted
  11  the step cap was reached before the checkpoint
  12  simulated time ran out before a terminal state
  13  the run completed but lost a syscall-wake response
  14  a --save-observation or --save-boot-summary artifact could not
      be produced
```

#### `cellgov boot bench`

Boot a title several times and gate the set against its anchor.

```console
$ cellgov boot bench --title flow --fw 4.93
$ cellgov boot bench --title wipeout --fw 4.93 --runs 5 --no-anchor-check
$ cellgov boot bench --all --runs 1
```

```
Usage: cellgov boot bench [OPTIONS] <--title <NAME>|--content-id <ID>|--title-manifest <PATH>|--all>
```

| Option | Value | Description |
| --- | --- | --- |
| `--title` | `NAME` | Short name from the title registry. |
| `--content-id` | `ID` | Content id (serial) from the title registry. |
| `--title-manifest` | `PATH` | A title manifest outside the registry. |
| `--fw` | `VERSION` | Installed firmware version. Without it, a disc title boots the firmware its record says it shipped with. Any other title, or a disc whose record names none, boots the only installed firmware. With none or several installed, the store refuses. |
| `--game-ver` | `base\|VERSION` | Installed content version; may be omitted when exactly one is a candidate. |
| `--firmware-dir` | `DIR` | A `sys/external` tree outside the store. Marks the run unmanaged, so it carries no firmware version. |
| `--max-steps` | `N` | Step cap; defaults to the cap the anchor was recorded at. |
| `--budget` | `N` | Simulated-time budget for the run. |
| `--checkpoint` | `process-exit\|first-rsx-write\|pc=0xADDR` | Stop condition, overriding the manifest's. |
| `--prescan` | -- | Scan the title for unimplemented opcodes before booting. |
| `--strict-reserved` | -- | Fault on a read of a reserved region instead of serving zeroes. |
| `--guest-arg` | `VALUE` | One guest argv entry; repeat for more. Values may spell a flag. |
| `--save-state-trace` | `PATH` | Write the run's state trace here. It records a state hash per step, which makes the run a divergence diagnostic instead of a throughput measurement. |
| `--run-index` | `N` | Index this measurement reports on its `BENCH_RESULT` line. A run set stamps each of its children. |
| `--skip-module-start` | -- | Run no firmware module's module_start in the boot process. |
| `--force-system-authid` | -- | Serve the system-class bdj.self program authority id instead of the one the title's SELF names. |
| `--prx-base` | `HEX` | Load the firmware module set at this 64K-aligned base inside the main region, instead of the first 64K page past the title image. A spawned child's firmware set loads at the same base. |
| `--disable-module-start-hle-stubs` | -- | Run the LLE path of each module_start the boot stubs to CELL_OK. |
| `--all` | -- | Gate every declared cell of every registry title, one after another; `--fw` / `--game-ver` narrow the cells. |
| `--no-anchor-check` | -- | Drop the anchor gate for a measurement-only run. |
| `--runs` | `N` | Subprocess measurements to take. With `1` the determinism gate compares nothing, and the set reports that. Default `3`. |
| `--strict-perf` | -- | Fail when the runs reach no throughput verdict. Use it only on a host that runs nothing else; elsewhere the spread measures the host. |

```
Exit codes particular to this command:
  15  --strict-perf is set and the run set reaches no throughput
      verdict

With --all, every declared cell of every registry title runs in turn,
one summary line each, and the status is the worst cell's: 3 when a
set broke determinism, 5 when a cell moved off its anchor, 4 when a
cell's boot failed, 15 as above, 1 when a declared cell has no anchor
or no cell ran at all. A cell the registry declares pending, or whose
firmware or dump is not installed, is reported by name and gates
nothing.
```

#### `cellgov boot bench-once`

One bench measurement, with no run set around it and no anchor gate.

```console
$ cellgov boot bench-once --title flow --fw 4.93 --max-steps 50000
```

```
Usage: cellgov boot bench-once [OPTIONS] <--title <NAME>|--content-id <ID>|--title-manifest <PATH>>
```

| Option | Value | Description |
| --- | --- | --- |
| `--title` | `NAME` | Short name from the title registry. |
| `--content-id` | `ID` | Content id (serial) from the title registry. |
| `--title-manifest` | `PATH` | A title manifest outside the registry. |
| `--fw` | `VERSION` | Installed firmware version. Without it, a disc title boots the firmware its record says it shipped with. Any other title, or a disc whose record names none, boots the only installed firmware. With none or several installed, the store refuses. |
| `--game-ver` | `base\|VERSION` | Installed content version; may be omitted when exactly one is a candidate. |
| `--firmware-dir` | `DIR` | A `sys/external` tree outside the store. Marks the run unmanaged, so it carries no firmware version. |
| `--max-steps` | `N` | Step cap; defaults to the cap the anchor was recorded at. |
| `--budget` | `N` | Simulated-time budget for the run. |
| `--checkpoint` | `process-exit\|first-rsx-write\|pc=0xADDR` | Stop condition, overriding the manifest's. |
| `--prescan` | -- | Scan the title for unimplemented opcodes before booting. |
| `--strict-reserved` | -- | Fault on a read of a reserved region instead of serving zeroes. |
| `--guest-arg` | `VALUE` | One guest argv entry; repeat for more. Values may spell a flag. |
| `--save-state-trace` | `PATH` | Write the run's state trace here. It records a state hash per step, which makes the run a divergence diagnostic instead of a throughput measurement. |
| `--run-index` | `N` | Index this measurement reports on its `BENCH_RESULT` line. A run set stamps each of its children. |
| `--skip-module-start` | -- | Run no firmware module's module_start in the boot process. |
| `--force-system-authid` | -- | Serve the system-class bdj.self program authority id instead of the one the title's SELF names. |
| `--prx-base` | `HEX` | Load the firmware module set at this 64K-aligned base inside the main region, instead of the first 64K page past the title image. A spawned child's firmware set loads at the same base. |
| `--disable-module-start-hle-stubs` | -- | Run the LLE path of each module_start the boot stubs to CELL_OK. |

### `cellgov diff`

Compare two runs.

```
Usage: cellgov diff [OPTIONS] <COMMAND>
```

#### `cellgov diff compare`

Run a scenario or manifest and compare it against a baseline.

```console
$ cellgov diff compare fairness
$ cellgov diff compare fairness --save-baseline fairness.baseline.json
$ cellgov diff compare fairness --against-baseline fairness.baseline.json --mode strict
```

```
Usage: cellgov diff compare [OPTIONS] <scenario|manifest.toml>
```

| Argument | Description |
| --- | --- |
| `scenario\|manifest.toml` | A scenario name, or a comparison manifest. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--mode` | `MODE` | How strictly the two sides must agree. One of `strict`, `memory`, `events`, `prefix`. Default `memory`. |
| `--save-baseline` | `PATH` | Record the scenario's observation here instead of comparing. |
| `--against-baseline` | `PATH` | Compare the scenario against this recorded baseline. |
| `--observations-dir` | `DIR` | Compare every observation in this directory; manifest targets only. |

```
Exit codes:
  0   the two runs agreed and no comparison diverged, or a plain run's
      manifest names no scenario this runner has (reported UNSUPPORTED)
  1   neither run produced an observation, a file failed to load or
      save, a baseline flag was given a manifest this runner cannot
      run, the comparison found a divergence, or with
      --observations-dir the baselines disagreed with each other
      (UNSETTLED_ORACLE)
  3   the two runs that had to reproduce each other disagreed, on a
      field or on whether an observation exists at all
```

#### `cellgov diff observations`

Diff two saved observation JSONs.

```console
$ cellgov diff observations cellgov.json rpcs3.json
```

```
Usage: cellgov diff observations [OPTIONS] <A.json> <B.json>
```

| Argument | Description |
| --- | --- |
| `A.json` | First observation JSON. Required. |
| `B.json` | Second observation JSON. Required. |

#### `cellgov diff diverge`

Report where two state captures first disagree.

```console
$ cellgov diff diverge cellgov.state rpcs3.state
```

```
Usage: cellgov diff diverge [OPTIONS] <A.state> <B.state>
```

| Argument | Description |
| --- | --- |
| `A.state` | First state capture. Required. |
| `B.state` | Second state capture. Required. |

```
Exit codes particular to this command:
  31  a trace failed to decode, so nothing past the cut was compared
```

#### `cellgov diff zoom`

Show one step's register-level diff between two zoom captures.

```console
$ cellgov diff zoom cellgov.zoom.state rpcs3.zoom.state 0x1f4
```

```
Usage: cellgov diff zoom [OPTIONS] <A.zoom.state> <B.zoom.state> <STEP>
```

| Argument | Description |
| --- | --- |
| `A.zoom.state` | First zoom capture. Required. |
| `B.zoom.state` | Second zoom capture. Required. |
| `STEP` | Step to zoom into; `0x` for hex. Required. |

```
Exit codes particular to this command:
  30  neither window covers the requested step
  31  a zoom trace failed to decode
```

### `cellgov explore`

Explore a scenario's schedule space.

```console
$ cellgov explore fairness
$ cellgov explore dma --format json
```

```
Usage: cellgov explore [OPTIONS] <SCENARIO>
       cellgov explore <COMMAND>
```

| Argument | Description |
| --- | --- |
| `SCENARIO` | Scenario to explore. Required. |

#### `cellgov explore micro`

Explore an LV2-driven micro-test.

```console
$ cellgov explore micro barrier_wakeup
$ cellgov explore micro mailbox_roundtrip --observations-dir observations/
```

```
Usage: cellgov explore micro [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `NAME` | Micro-test name. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--observations-dir` | `DIR` | Compare each schedule against the observations here. |

#### `cellgov explore title`

Explore a window of a composed title boot.

```console
$ cellgov explore title --title flow --fw 1.50
$ cellgov explore title --title sshd --fw 4.93 --start-step 20000
$ cellgov explore title --title wipeout --fw 4.93 --max-schedules 32 --format json
```

```
Usage: cellgov explore title [OPTIONS] <--title <NAME>|--content-id <ID>|--title-manifest <PATH>>
```

| Option | Value | Description |
| --- | --- | --- |
| `--title` | `NAME` | Short name from the title registry. |
| `--content-id` | `ID` | Content id (serial) from the title registry. |
| `--title-manifest` | `PATH` | A title manifest outside the registry. |
| `--fw` | `VERSION` | Installed firmware version. Without it, a disc title boots the firmware its record says it shipped with. Any other title, or a disc whose record names none, boots the only installed firmware. With none or several installed, the store refuses. |
| `--game-ver` | `base\|VERSION` | Installed content version; may be omitted when exactly one is a candidate. |
| `--firmware-dir` | `DIR` | A `sys/external` tree outside the store. Marks the run unmanaged, so it carries no firmware version. |
| `--max-steps` | `N` | Retired-instruction cap for the whole boot, the window included; defaults to the cap the cell's anchor was recorded at. |
| `--max-schedules` | `N` | Explore at most this many alternate schedules. Default `256`. |
| `--max-steps-per-run` | `N` | Take at most this many runtime steps per replayed schedule. Default `10000`. |
| `--start-step` | `N` | Open the window after this many runtime steps. Without it and without --start-pc, the window opens at the first step two units are runnable at. |
| `--start-pc` | `HEX` | Open the window once a step yields at this guest PC. A PC reached inside a batch never matches. |

```
Exit codes particular to this command:
  20  the model refused a schedule it was asked to explore: a refused
      commit, or a refused step. The cell's own first-rsx-write
      checkpoint is not one of them.
  21  the window never opened: the boot reached a terminal state, a
      cap, a refusal or a fault before the start condition
  22  a schedule the exploration ran ended in a guest fault. Distinct
      from 20: the guest's own step failed rather than the model
      declining one. A refusal outranks it.
  23  the window holds a spawn whose staged init pass no exploration
      runs, so the search answers for none of it. Nothing is wrong with
      the model; start the window after the spawn or end it before.

A schedule-sensitive window -- two schedules that both ran themselves
out committed different memory -- takes the shared status 1. A cap the
caller set, and a window whose units all blocked, report inconclusive
and exit 0.
```

### `cellgov scenario`

Run a synthetic SPU/PPU scenario.

```
Usage: cellgov scenario [OPTIONS] <COMMAND>
```

#### `cellgov scenario list`

Name every synthetic scenario.

```console
$ cellgov scenario list
```

```
Usage: cellgov scenario list [OPTIONS]
```

#### `cellgov scenario run`

Run one scenario and print its report.

```console
$ cellgov scenario run fairness
```

```
Usage: cellgov scenario run [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `NAME` | Scenario name. Required. |

#### `cellgov scenario dump`

Run one scenario and print every trace record.

```console
$ cellgov scenario dump dma
```

```
Usage: cellgov scenario dump [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `NAME` | Scenario name. Required. |

### `cellgov dev`

Maintainer tooling.

```
Usage: cellgov dev [OPTIONS] <COMMAND>
```

#### `cellgov dev disasm`

Disassemble a guest ELF at a virtual address.

```console
$ cellgov dev disasm vfs/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN --vaddr 0x10381ce8
$ cellgov dev disasm dumps/flow.elf --vaddr 0x10000 --count 64 --symbolize
```

```
Usage: cellgov dev disasm [OPTIONS] --vaddr <HEX> <ELF>
```

| Argument | Description |
| --- | --- |
| `ELF` | Guest ELF, PRX, or SELF. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--vaddr` | `HEX` | Virtual address to start at; must be 4-byte aligned. Required. |
| `--count` | `N` | Instruction count. Default `16`. |
| `--symbolize` | -- | Build the OPD function map and annotate branch targets. |

```
SCE-wrapped input:
  this build has no decrypt support: plaintext ELF / PRX only. An
  SCE-wrapped input is refused by name, and --vfs-root names no path
  this build reads; rebuild with --features decrypt to read one.

Exit codes particular to this command:
  20   at least one word decoded to no instruction
  141  stdout was closed by a downstream reader
```

#### `cellgov dev prx-imports`

Print a PRX or executable's import table.

```console
$ cellgov dev prx-imports vfs/dev_flash/sys/external/libsysmodule.sprx
$ cellgov dev prx-imports dumps/EBOOT.BIN --module cellGcmSys
```

```
Usage: cellgov dev prx-imports [OPTIONS] <PATH>
```

| Argument | Description |
| --- | --- |
| `PATH` | A `.prx`, `.sprx`, or title executable. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--at` | `HEX` | Show only the import whose stub sits at this file-relative address. |
| `--module` | `NAME` | Show only imports from this module. |
| `--save-elf` | `PATH` | Write the decrypted plaintext ELF here. |

```
SCE-wrapped input:
  this build has no decrypt support: plaintext ELF / PRX only. An
  SCE-wrapped input is refused by name, and --vfs-root names no path
  this build reads; rebuild with --features decrypt to read one.
```

#### `cellgov dev funcs`

Print the OPD-derived function map for an ELF or PRX.

```console
$ cellgov dev funcs vfs/dev_flash/sys/external/liblv2.sprx
$ cellgov dev funcs dumps/EBOOT.BIN --json
```

```
Usage: cellgov dev funcs [OPTIONS] <ELF>
```

| Argument | Description |
| --- | --- |
| `ELF` | Guest ELF, PRX, or SELF. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--json` | -- | Emit the map as JSON instead of a table. |

```
SCE-wrapped input:
  this build has no decrypt support: plaintext ELF / PRX only. An
  SCE-wrapped input is refused by name, and --vfs-root names no path
  this build reads; rebuild with --features decrypt to read one.
```

#### `cellgov dev lv2-discover`

Locate the syscall dispatch table in a decrypted LV2 kernel.

```console
$ cellgov dev lv2-discover ../cellgov-output/lv2_kernel-3.55.elf
$ cellgov dev lv2-discover ../cellgov-output/lv2_kernel-3.55.elf --format json
```

```
Usage: cellgov dev lv2-discover [OPTIONS] <ELF>
```

| Argument | Description |
| --- | --- |
| `ELF` | Decrypted LV2 kernel ELF, or an SCE wrapper in a decrypt build. Required. |

```
SCE-wrapped input:
  this build has no decrypt support: plaintext ELF / PRX only. An
  SCE-wrapped input is refused by name, and --vfs-root names no path
  this build reads; rebuild with --features decrypt to read one.
```

#### `cellgov dev lv2-census`

Emit the LV2 census rows for one firmware version.

```console
$ cellgov dev lv2-census ../cellgov-output/lv2_kernel-3.55.elf --fw 3.55 --pup-sha256 334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192
$ cellgov dev lv2-census ../cellgov-output/lv2_kernel-3.55.elf --fw 3.55 --pup-sha256 334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192 --replace-version
```

```
Usage: cellgov dev lv2-census [OPTIONS] --fw <VERSION> --pup-sha256 <SHA256> <ELF>
```

| Argument | Description |
| --- | --- |
| `ELF` | Decrypted kernel ELF, or a SELF the configured vault can open. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--fw` | `VERSION` | Select the firmware version from `pup.tsv`. Required. |
| `--pup-sha256` | `SHA256` | Use the source PUP's SHA-256 from `pup.tsv`. Required. |
| `--output-dir` | `DIR` | Write the archive rows to this directory. Default `docs/lv2`. |
| `--replace-version` | -- | Replace all rows for this firmware with rows from the selected PUP. |

#### `cellgov dev rpcs3-attribute`

Answer which HLE call wrote a guest address, from a trace.

```console
$ cellgov dev rpcs3-attribute --trace hle.trace --addr 0x10000000 --len 0x40
$ cellgov dev rpcs3-attribute --trace hle.trace --ranked
```

```
Usage: cellgov dev rpcs3-attribute [OPTIONS] --trace <PATH> <--addr <HEX>|--list|--ranked|--name <SUBSTR>>
```

| Option | Value | Description |
| --- | --- | --- |
| `--trace` | `PATH` | The HLE trace to read. Required. |
| `--addr` | `HEX` | Report the calls that wrote this guest address. |
| `--len` | `HEX` | Bytes covered from `--addr`, hex like the address (default 1). |
| `--list` | -- | List every call in the trace. |
| `--ranked` | -- | Rank the calls by how much they wrote. |
| `--name` | `SUBSTR` | Report only calls whose name contains this substring. |

#### `cellgov dev fixture-gen`

Regenerate a title's cross-runner fixture directory.

```console
$ cellgov dev fixture-gen --manifest title_manifests/NPUA80001.toml --cellgov cellgov.json --rpcs3 rpcs3.json --fw 4.93 --game-ver base
```

```
Usage: cellgov dev fixture-gen [OPTIONS] --manifest <PATH> --cellgov <PATH> --rpcs3 <PATH>
```

| Option | Value | Description |
| --- | --- | --- |
| `--manifest` | `PATH` | The title manifest the fixture is generated for. Required. |
| `--cellgov` | `PATH` | CellGov's observation JSON. Required. |
| `--rpcs3` | `PATH` | The other runner's observation JSON. Required. |
| `--fixtures-dir` | `DIR` | Fixture tree the cell's directory is created under. |
| `--allow-divergence` | -- | Write the fixture even when the two observations disagree. |
| `--fw` | `VERSION` | Installed firmware version. Without it, a disc title boots the firmware its record says it shipped with. Any other title, or a disc whose record names none, boots the only installed firmware. With none or several installed, the store refuses. |
| `--game-ver` | `base\|VERSION` | Installed content version; may be omitted when exactly one is a candidate. |
| `--firmware-dir` | `DIR` | A `sys/external` tree outside the store. Marks the run unmanaged, so it carries no firmware version. |

#### `cellgov dev titles-gen`

Regenerate the title documents from the registry and fixtures.

```console
$ cellgov dev titles-gen
```

```
Usage: cellgov dev titles-gen [OPTIONS]
```

| Option | Value | Description |
| --- | --- | --- |
| `--registry` | `DIR` | Title registry directory. |
| `--fixtures-dir` | `DIR` | Cross-runner fixture directory. |
| `--output-dir` | `DIR` | Directory the generated documents are written under. |

#### `cellgov dev cli-gen`

Regenerate `docs/cli.md` from this command tree.

```console
$ cellgov dev cli-gen
```

```
Usage: cellgov dev cli-gen [OPTIONS]
```

| Option | Value | Description |
| --- | --- | --- |
| `--output` | `PATH` | Document to write. |

#### `cellgov dev workspace-gen`

Regenerate Cargo-derived regions of `docs/architecture/workspace.md`.

```console
$ cellgov dev workspace-gen
```

```
Usage: cellgov dev workspace-gen [OPTIONS]
```

| Option | Value | Description |
| --- | --- | --- |
| `--output` | `PATH` | Architecture document to update. |

#### `cellgov dev completions`

Print a shell completion script for this command tree.

```console
$ cellgov dev completions bash
$ cellgov dev completions pwsh
```

```
Usage: cellgov dev completions [OPTIONS] <SHELL>
```

| Argument | Description |
| --- | --- |
| `SHELL` | Shell the script is written for. One of `bash`, `zsh`, `pwsh`. Required. |

```
The script goes to stdout; redirect it to where the shell reads it:

  bash  cellgov dev completions bash > ~/.local/share/bash-completion/completions/cellgov
  zsh   cellgov dev completions zsh > "${fpath[1]}/_cellgov"
  pwsh  cellgov dev completions pwsh >> $PROFILE

Exit codes particular to this command:
  141  stdout was closed by a downstream reader
```

#### `cellgov dev gen-manifest`

Emit a title-manifest stub from an install record.

```console
$ cellgov dev gen-manifest --title-id NPUA80001
$ cellgov dev gen-manifest --firmware 4.93
$ cellgov dev gen-manifest --record vfs/.cellgov/installs/titles/NPUA80001/base.install.toml --force
```

```
Usage: cellgov dev gen-manifest [OPTIONS] <--record <PATH>|--title-id <ID>|--firmware <VERSION>>
```

| Option | Value | Description |
| --- | --- | --- |
| `--record` | `PATH` | An install record to read directly. |
| `--title-id` | `ID` | A title id whose base record is looked up under `--installs`. |
| `--firmware` | `VERSION` | A firmware version whose record is looked up under `--installs`. |
| `--installs` | `DIR` | Install-records directory `--title-id` and `--firmware` are resolved under. |
| `--registry` | `DIR` | Registry directory the stub is written into. |
| `--force` | -- | Overwrite an existing manifest. |

```
Notes:
  A title's base record generates that title's manifest; a title-update
  record is refused by name. The stub's system_ver is read from the
  installed tree's PARAM.SFO. That tree, and the record directory
  --title-id and --firmware default to, sit under the store root
  --vfs-root names, so the tree must be present there. A firmware
  record generates the manifest for the system software the firmware
  ships. That manifest names no firmware version: the store holds the
  version, and the manifest resolves against whichever firmware --fw
  selects.
```

#### `cellgov dev record-anchors`

Re-measure titles and rewrite their committed anchors.

```console
$ cellgov dev record-anchors --title flow
$ cellgov dev record-anchors --all --fw 4.93
```

```
Usage: cellgov dev record-anchors [OPTIONS] <--all|--title <NAME>>
```

| Option | Value | Description |
| --- | --- | --- |
| `--all` | -- | Re-measure every title in the registry. |
| `--title` | `NAME` | Re-measure one title by short name. |
| `--fw` | `VERSION` | Record only the declared cells at this firmware version. |
| `--game-ver` | `base\|VERSION` | Record only the declared cells at this game version. |
| `--registry` | `DIR` | Registry directory; must be the one the measurement reads. |

#### `cellgov dev oracle-gap`

Build the local oracle-gap overlay from the operator checkout.

```console
$ cellgov dev oracle-gap
```

```
Usage: cellgov dev oracle-gap [OPTIONS]
```

#### `cellgov dev decoder-sweep`

Scan a bounded instruction-word range or an explicit full-domain shard.

```console
$ cellgov dev decoder-sweep ppu --count 65536 --output ppu-bounded.json
$ cellgov dev decoder-sweep spu --full --shard 0 --shards 16 --output spu-shard0.json
```

```
Usage: cellgov dev decoder-sweep [OPTIONS] --output <PATH> <--full|--count <COUNT>> <DECODER>
```

| Argument | Description |
| --- | --- |
| `DECODER` | Decoder whose outcomes to classify. One of `ppu`, `spu`. Required. |

| Option | Value | Description |
| --- | --- | --- |
| `--full` | -- | Explicitly scan one shard of the full 32-bit word space. |
| `--start` | `START` | First bounded word, in hexadecimal; zero when omitted. |
| `--count` | `COUNT` | Word count for a bounded scan. |
| `--shard` | `SHARD` | Zero-based full-domain shard index; zero when omitted. |
| `--shards` | `SHARDS` | Number of full-domain shards; one when omitted. |
| `--chunk-size` | `CHUNK_SIZE` | Maximum words per bounded chunk. Default `65536`. |
| `--workers` | `WORKERS` | Deterministic worker partition count. Default `1`. |
| `--cancel-after` | `CANCEL_AFTER` | Stop after this many words and mark the artifact cancelled. |
| `--output` | `PATH` | Write the versioned JSON result here. Required. |

Generated by `cellgov dev cli-gen`. Do not hand-edit; rerun the
generator after changing a command, a flag, or an example.
