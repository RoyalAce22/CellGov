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
divergent-gap count for its PRX closure reaches zero. Each
title loads exactly its transitive PRX closure -- the firmware
modules its binary imports, derived at boot rather than
hand-listed -- and that full-closure load is safe to attempt
precisely because the null backend makes a
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

- 3 games boot to deterministic checkpoints past the
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
- The PS3 system shell boots as a guest process, straight out
  of the firmware image with no install step, under nearly the
  complete firmware module set -- exports resolve under the
  library name each import carries, so modules that share NIDs
  coexist. It exercises paths no game reaches -- privileged
  module registration driven by the executable's own SELF
  capability header, runtime import linking, the
  event-port family, and the firmware's system-IPC key
  namespace -- and runs to a `MaxSteps` budget cap.
- PPU and SPU interpreters: complete decode for the PPC64 and
  SPU ABI surfaces titles in the current corpus exercise (see
  [docs/architecture.md](docs/architecture.md) for the current
  per-instruction surface).
- LV2: a growing set of classified syscalls, each handled arm
  carrying a reviewed fidelity tag in the drift-checked
  [docs/lv2_fidelity.md](docs/lv2_fidelity.md). Userspace
  surfaces load as firmware SPRX modules from the user's PUP.
  Unmodeled syscalls return an ABI-honest "not implemented"
  response via the null backend. Unresolved imports surface
  as named diagnostics via a guest-resident trampoline.
- Sync primitives (lwmutex, event flag, semaphore, mutex, cond, event queues and ports), filesystem with host-backed VFS, and PRX import inspection (`cellgov_cli dump-prx-imports`). Recursive mutexes carry real lock counts (owner relock succeeds, unlock releases only at zero), and create/join arms enforce the kernel's argument-validation contracts. Wait timeouts are honoured: a timed wait expires with `CELL_ETIMEDOUT` at its guest-tick deadline, and `usleep`/`sleep` deschedule the caller until the deadline arrives -- all through a deterministic timer-wake queue that is snapshot-captured and state-hashed.
- Multi-process: `sys_process_spawn` creates a real child process in
  its own address space, decrypting a child that ships as an
  encrypted SELF rather than a plain ELF, with per-process identity
  (getpid/getppid/exit status). Memory two processes map under the
  same IPC key is one segment -- a write through either mapping is
  visible through the other, and the mapping forms from the guest's
  own `sys_mmapper` calls with nothing declared on the host side.
  The address-space boundary holds across the rest of the kernel
  surface: a thread a child creates stacks inside the child, a map
  that would land on memory the caller already holds is refused with
  `CELL_EBUSY` (and stepped over when the kernel searches for a free
  window), and a wait result is written in the waiting thread's
  space, not the signaller's. A process exit drops its threads out
  of every waiter list, so no mutex, semaphore count, or join value
  is handed to a thread that will never run again. Schedule
  exploration compares child spaces too, so cross-process races are
  witnessed, not missed. A parent-spawns-child guest microtest runs
  the surface end to end.
- Real-firmware SELF decryption and loading from `PS3UPDAT.PUP`;
  every module a boot loads is verified against the install's
  manifest, so an altered or mismatched firmware corpus fails
  loudly instead of skewing the oracle.
- Per-title boot baselines are committed data (step counts,
  outcomes, and named behaviour witnesses), re-measured and
  blessed through a single recording command.
- Kernel-state hashing is complete by construction -- a host
  field cannot be added without an explicit fold decision --
  and the instrumentation layer is proven inert: wiping every
  witness after each step leaves the boot's state trace
  byte-identical.
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

Cargo workspace, 16 library crates and 3 binaries (+1 install tool). See
[`docs/architecture.md`](docs/architecture.md) for the layering
diagram and per-crate responsibilities.

## Building

Requires Rust 1.95 or newer.

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

CellGov has no runtime dependency on RPCS3. Booting a real PS3 game
requires PS3 system firmware. Download the official firmware update
(`PS3UPDAT.PUP`) from
[playstation.com](https://www.playstation.com/en-us/support/hardware/ps3/system-software/)
and install it with the included tool:

```bash
cargo run -p cellgov_install -- install /path/to/PS3UPDAT.PUP
```

The install unwraps the outer SCE/PUP envelope and writes per-module
SELFs under `vfs/dev_flash/` (gitignored; bytes are never vendored). Each
SELF stays encrypted on disk and is decrypted at boot time. `run-game`
auto-discovers the install: `--firmware-dir` defaults to
`vfs/dev_flash/sys/external/` when that directory exists at the current
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
