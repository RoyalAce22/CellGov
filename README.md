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

- **Titles**: 3 boot to cross-runner checkpoints -- flOw, Super
  Stardust HD, WipEout HD Fury. See [docs/titles.md](docs/titles.md).
- **PPU**: 160 `PpuInstruction` variants (architectural ops plus
  shadow quickenings and super-pair fusions; covers the
  CR-logical XL-form family and the X-form FP indexed load/store
  family). **SPU**: full interpreter.
- **LV2**: 62 classified syscalls, 56 HLE exports, including the
  `sys_rsx` kernel-side RSX surface, the cellSysutil video-out
  query family, the cellSpurs PPU-side runtime (initialize,
  workload registry, ready-count and contention controls, info
  snapshot, exception-handler registration), HLE sys_lwmutex_*
  routing through the LV2 lwmutex surface with a kernel
  sleep-queue, recursive-acquire depth tracking, and
  per-thread hold counts that drive critical-section-aware
  scheduling so concurrent printf paths complete atomically,
  and the read-side `sys_fs_*` family
  (open / read / close / lseek / fstat / stat) backed by an
  in-memory blob store that titles populate via per-title
  content manifests.
- **Sync primitives**: lwmutex / event_flag / semaphore / mutex
  / cond now match real-PS3 wake ordering, including event_flag
  cancel waking parked waiters with `CELL_ECANCELED`,
  semaphore post-N multi-wake, finite-timeout waits trip
  `ETIMEDOUT` only when no peer can satisfy them, and timer
  syscalls advance the deterministic guest clock. Critical-
  section sticky scheduling is bounded by a sticky-streak
  ceiling so a single thread holding an lwmutex cannot starve
  peers indefinitely.
- **In-memory FS**: the LV2 `sys_fs_*` surface is backed by a
  path-keyed blob store with a never-recycle fd allocator and
  deterministic 56-byte `CellFsStat` (zero atime / mtime /
  ctime, S_IFREG read-only mode, 4096 blksize). Titles populate
  it from a per-title content manifest with three-tier
  resolution (env override -> EBOOT-adjacent USRDIR
  auto-discovery -> manifest's checked-in synthetic stubs).
  Both raw `sys_fs_*` syscalls and the `cellFs*` HLE wrappers
  route through the same store.
- **PS3 conformance**: ps3autotests cross-runner harness boots
  whitelisted .ppu.elf files and compares captured TTY against
  real-PS3 .expected. cpu/basic, cpu/ppu_branch,
  lv2/sys_process, lv2/sys_semaphore, lv2/sys_event_flag all
  match byte-for-byte.
- 2,470 tests, zero `unsafe` (`unsafe_code = forbid`).

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
requires PS3 system firmware (decrypted SPRX modules like
`liblv2.sprx`). Download the official firmware update
(`PS3UPDAT.PUP`) from
[playstation.com](https://www.playstation.com/en-us/support/hardware/ps3/system-software/)
and decrypt it with the included tool:

```bash
cargo run -p cellgov_firmware -- install PS3UPDAT.PUP --output dev_flash
```

Then pass `--firmware-dir dev_flash/sys/external` to `run-game`.

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
