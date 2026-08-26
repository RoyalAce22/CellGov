# Microtest corpus

PSL1GHT-compiled C microtests live under `tests/micro/<name>/`,
each with its own `manifest.toml` and `build.sh`, and run
end-to-end as LV2-driven scenarios from the PPU's own compiled
code. SPU-bearing tests drive the full SPU lifecycle through
syscalls with no harness pre-registration of SPU execution units;
PPU-only (`ppu_*`) and RSX-focused (`rsx_*`) microtests exercise
those subsystems without SPU threads. A representative selection:

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

Each test has interpreter and LLVM scenario observations under
`tests/scenario_observations/`, settled when both decoders agree.
CellGov runs each through `observe_with_determinism_check` (two
runs must produce identical results) and compares against both
via `compare_multi --mode memory`.
