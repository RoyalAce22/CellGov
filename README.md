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

## The null backend

CellGov loads the firmware PRXes a title needs and models
their LV2 syscalls to RPCS3-faithful behavior. The set of
syscalls a loaded PRX exercises is large; not all of them are
modeled yet. The policy for the unmodeled gap is the **null
backend**: every syscall a loaded PRX makes that CellGov has
not modeled yet returns an ABI-honest, per-syscall, traced
"not implemented" response (`CELL_ENOSYS` and similar --
never a blanket `CELL_OK`). The consequence: every
cross-runner divergence is an implementation target the
oracle named, not a failure of the oracle.

The titles matrix is a frontier map of the unimplemented
syscall surface, not a pass/fail scoreboard; each `No` row
identifies the specific firmware path whose modeling closes
the divergence. A title transitions from "boots-with-
honest-gaps" to "boots-clean (converges)" when the
divergent-gap count for its PRX closure reaches zero. The
current "minimum PRX set" is scaffolding and not the final goal -- it
dissolves title-by-title as syscall coverage grows, and
loading a title's full transitive PRX closure becomes safe
to attempt precisely because the null backend makes a
premature load fail honestly (named divergence) instead of
silently (fabricated success). See
[docs/concepts.md](docs/concepts.md) for the honest /
contaminating / convergent / divergent vocabulary the matrix
uses.

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

- 3 titles boot to deterministic checkpoints past the
  firmware `cellSysutil` init. WipEout HD Fury reaches
  `FirstRsxWrite` and converges with RPCS3 at that
  checkpoint (byte parity `975 non-semantic + 1 pending`);
  flOw runs the full firmware-set boot to `sys_process_exit`,
  and Super Stardust HD runs to a `MaxSteps` budget cap.
  flOw and Super Stardust HD diverge from RPCS3, which keeps
  executing past CellGov's stopping point; each divergence
  names the specific unmodeled syscall as the next
  implementation target (see "The null backend" above and
  [docs/titles.md](docs/titles.md)).
- PPU and SPU interpreters: complete decode for the PPC64 and
  SPU ABI surfaces titles in the current corpus exercise;
  coverage grows per phase (see
  [docs/architecture.md](docs/architecture.md) for the current
  per-instruction surface).
- LV2: a growing set of classified syscalls (numbers shift
  every phase as PRX coverage grows; see
  [docs/architecture.md](docs/architecture.md)). Userspace
  surfaces load as firmware SPRX modules from the user's PUP.
  Unmodeled syscalls return an ABI-honest "not implemented"
  response via the null backend. Unresolved imports surface
  as named diagnostics via a guest-resident trampoline.
- Sync primitives (lwmutex, event flag, semaphore, mutex, cond), filesystem with host-backed VFS, and PRX import inspection (`cellgov_cli dump-prx-imports`).
- Real-firmware SELF decryption and loading from `PS3UPDAT.PUP`.
- ps3autotests cross-runner harness present.
- Workspace test suite green in debug and release; zero
  `unsafe` (`unsafe_code = forbid`); strict clippy gate.

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
