# CellGov

[![CI](https://img.shields.io/github/actions/workflow/status/RoyalAce22/CellGov/ci.yml?branch=main&label=CI)](https://github.com/RoyalAce22/CellGov/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.95-orange.svg)](https://blog.rust-lang.org/2026/04/03/Rust-1.95.0.html)
[![Status: pre-alpha](https://img.shields.io/badge/status-pre--alpha-orange.svg)](#status)

## What CellGov is

CellGov interprets PS3 PPU and SPU code deterministically, produces
replayable execution traces, and validates its output against RPCS3
baselines. It is the **foundation layer for static recompilation** of
PS3 games to native binaries: not the recompiler itself, but the oracle
that tells the recompiler what the correct output is and which
synchronization patterns it must preserve.

The design rule at the center:

> Determinism comes from one rule: nothing a thread does is "live"
> until the runtime says so. Threads propose changes; the runtime is
> the only thing that ever applies them.

## What CellGov is not

CellGov does not run games. There is no RSX rasterisation, no vblank,
no audio, no networking, no input, no JIT, no host-speed execution,
and no per-title compatibility hacks. RPCS3 is the right tool to play
a game. CellGov is the right tool to ask, byte-for-byte, what a PS3
game would produce under any legal schedule.

## Why determinism matters for static recomp

PS3 games run PPU and SPU threads concurrently. A static recompiler
must decide which synchronization to preserve and which is incidental.
CellGov answers that question:

- **Deterministic tracing.** Two runs of the same scenario produce
  byte-identical traces of every scheduling decision, effect, and
  commit.
- **Schedule exploration.** Bounded enumeration of legal interleavings,
  classified by whether they produce different observable outcomes.
- **Oracle comparison.** A normalized observation schema lets CellGov
  cross-validate against RPCS3, and lets a downstream recompiler
  cross-validate its output against CellGov.

## Status

Pre-Alpha. What works today:

- **3 titles boot to cross-runner checkpoints**: flOw, Super Stardust HD, WipEout HD Fury (see [docs/titles.md](docs/titles.md)).
- **PPU and SPU interpreters**: 160 PPU instructions; full SPU.
- **LV2**: 84 classified syscalls. The userspace surfaces (cellSpurs, cellSysutil, cellGcmSys, cellSaveData, sysPrxForUser, cellFs) are loaded as firmware SPRX modules from the user's PUP install -- CellGov does not carry a Rust HLE reimplementation.
- **Sync primitives**: lwmutex / event_flag / semaphore / mutex / cond match real-PS3 wake ordering (incl. ETIMEDOUT and CELL_ECANCELED paths).
- **FS with host-backed VFS**: path-keyed blob store backs `sys_fs_*` calls; per-title mounts (default `/app_home`) resolve guest paths against host directories with deterministic lexicographic enumeration. Directory iteration syscalls (opendir / readdir / closedir) operate on snapshot entries. Kernel fd allocation matches real PS3's `[3, 255)` range.
- **Real-firmware SELF decryption + loading**: `cellgov_firmware install` peels a user-supplied `PS3UPDAT.PUP` into per-module SELFs; each one is decrypted on the fly at boot time and loaded into guest memory with PPC64 relocations applied as one atomic batch (ADDR32, ADDR64, ADDR16_LO/HI/HA/LO_DS, REL24). The minimum viable PRX set loads in topological-sort order with `module_start` invoked per module under a synthetic kernel-context OPD; the firmware identity contributes to the LV2 state hash. `run-game --boot-mode firmware-set` opts the default boot path into the firmware loader. Fourteen SPRX modules from the user's PUP decrypt bit-identically to RPCS3's decrypter output. APP keys cover firmware revisions 0x0000-0x001D.
- **PRX import inspection**: `cellgov_cli dump-prx-imports <path>` decodes any raw `.prx` or SCE-wrapped `.sprx` and prints the module's internal name, export namespaces, and full import table; auto-detects SCE wrappers and decrypts via `cellgov_firmware::sce`.
- **PS3 conformance**: ps3autotests cross-runner harness is present; the 6 integration tests (cpu/basic, cpu/ppu_branch, lv2/sys_process, lv2/sys_semaphore, lv2/sys_event_flag, plus a determinism double-run) are currently `#[ignore]`'d pending rewire to firmware-set boot or filling in the LV2 syscall coverage their default single-PRX boot path needs.
- 3,185 tests, zero `unsafe` (`unsafe_code = forbid`).

See [docs/architecture.md](docs/architecture.md) for the full pipeline, memory model, and per-subsystem details.

### Next reads

- [docs/concepts.md](docs/concepts.md) -- what CellGov produces
  (observations, checkpoints, cross-runner agreement) and the
  vocabulary the rest of the docs use. Read this first.
- [docs/titles.md](docs/titles.md) -- compatibility matrix.
- [docs/architecture.md](docs/architecture.md) -- pipeline,
  memory model, effect vocabulary, per-crate responsibilities.

## Workspace

Cargo workspace, 16 library crates and 3 binaries (+1 firmware tool). See
[`docs/architecture.md`](docs/architecture.md) for the layering
diagram and per-crate responsibilities.

## Building

Requires Rust 1.95 or newer.

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

CellGov has no runtime dependency on RPCS3. Booting a real PS3 game
requires PS3 system firmware. Download the official firmware update
(`PS3UPDAT.PUP`) from
[playstation.com](https://www.playstation.com/en-us/support/hardware/ps3/system-software/)
and install it with the included tool:

```bash
cargo run -p cellgov_firmware -- install /path/to/PS3UPDAT.PUP
```

The install unwraps the outer SCE/PUP envelope and writes per-module
SELFs under `firmware/` (gitignored; bytes are never vendored). Each
SELF stays encrypted on disk and is decrypted at boot time. `run-game`
auto-discovers the install: `--firmware-dir` defaults to
`firmware/sys/external/` when that directory exists at the current
working directory; pass `--firmware-dir DIR` to override or set
`CELLGOV_NO_FIRMWARE_DIR=1` to suppress the default.

The `cellgov_compare` crate gates the RPCS3 process-spawning runner
behind the default-on `rpcs3-runner` Cargo feature. Importers that
just want the `Observation` schema, `compare()`, `diverge()`, and
`zoom_lookup()` can opt out with
`default-features = false` and never compile RPCS3-aware code.

## Testing

Test assertions run against structured trace records and final state
hashes, never against human-readable logs. The comparison harness
validates CellGov observations against RPCS3 baselines through a
runner-agnostic observation schema.

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT license
