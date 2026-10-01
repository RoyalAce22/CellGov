# Microtest suite

**PSL1GHT-compiled C microtests run end-to-end as LV2-driven
scenarios from the PPU's own compiled code.** They live under
`tests/micro/<name>/`, each with its own `manifest.toml` and
`build.sh`. Kinds of test include:

- SPU-bearing tests, which drive the full SPU lifecycle through
  syscalls, with no harness pre-registration of SPU execution units;
- PPU-only (`ppu_*`) and RSX-focused (`rsx_*`) microtests, which
  exercise those subsystems without SPU threads.

## Representative tests

A representative selection:

| Test                   | What it proves                                                 |
| ---------------------- | -------------------------------------------------------------- |
| spu_fixed_value        | SPU writes a known value via DMA put.                          |
| mailbox_roundtrip      | PPU-to-SPU mailbox send, SPU transforms and DMA puts result.   |
| dma_completion         | 128-byte DMA put with tag wait, status header.                 |
| atomic_reservation     | SPU `getllar` / `putllc` (load-linked, store-conditional).     |
| ls_to_shared           | Dependent LS store-to-load chain published via DMA.            |
| barrier_wakeup         | Two SPU threads, inter-SPU ordering via shared memory polling. |
| ppu_atomic_spinlock    | PPU `lwarx`/`stwcx.` spin acquire under contention.            |
| ppu_cond_prodcons      | sys_cond producer-consumer with mutex two-hop reacquire.       |
| ppu_event_flag_wakeall | sys_event_flag bitmask AND/OR wake fan-out.                    |
| ppu_lwmutex_counter    | lwmutex contention on a shared counter.                        |
| rsx_label_write_poll   | RSX label byte transition observed via PPU spin-poll.          |
| rsx_semaphore_post     | NV4097 semaphore release polled by PPU.                        |
| process_spawn_wait     | Parent spawns an SCE-wrapped child into its own address space and waits while the child runs a PPU thread on a stack there. |

See `tests/micro/` for the full set.

## Toolchain

**`tests/micro/toolchain/Dockerfile` pins the toolchain that builds
the microtests.** It wraps a named ps3dev nightly release (PPU and SPU
gcc, PSL1GHT, make_self).

Each `build.sh` header contains the `docker build` and `docker run`
recipe.

## Observations

**Three runners answer a microtest: CellGov, RPCS3 and a retail
console.** Where the console has answered, its capture is the
reference; the emulator observations are peers.

- **Hardware capture.** A console's answer, committed under
  `tests/micro/<name>/ps3/<profile>/`. The reference is the capture
  under the reference profile named in
  `tests/micro/console_profiles.toml`; a capture under any other
  profile is a peer.
- **Scenario observations.** RPCS3's interpreter and LLVM answers,
  under `tests/scenario_observations/`. They are settled when both
  decoders agree, and an emulator answer that differs from the
  console's is classified, never copied into CellGov.

A test no runner can run is checked against the values its documented
layout fixes.

CellGov checks each test in two steps:

1. Run it through `observe_with_determinism_check` (two runs must
   produce identical results).
2. Compare the result with its reference in memory: the console
   capture, with the manifest's `[ps3] volatile` bytes blanked on both
   sides, or, for a test with no capture yet, both emulator
   observations via `compare_multi`.
