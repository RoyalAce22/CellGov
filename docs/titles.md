# Tested Titles

Boot-frontier compatibility matrix. Each row is a title CellGov boots
to a checkpoint whose observation is byte-equivalent (modulo
classified non-semantic divergences) to an RPCS3 baseline at the
same checkpoint.

This is not gameplay compatibility. See
[architecture.md](architecture.md) for what CellGov does and does
not model.

## Reading the table

Terminology used in the Cross-runner column comes from
[concepts.md](concepts.md). In brief:

- `equivalent (N bytes non-semantic)` means the raw byte-level
  comparison found N differing bytes, every one of which has been
  classified as not affecting program behaviour (HLE OPD layout,
  SELF reconstruction metadata, etc.). The raw
  `tests/fixtures/<serial>_cross_runner/compare_report.txt`
  contains the `DIVERGE` lines for those bytes plus the narrative
  that classifies each one.
- `DIVERGE` (without the `equivalent` qualifier) would mean the
  divergence is either unclassified or includes at least one
  semantic difference. No title currently in the matrix sits in
  that state.

Column definitions:

- **Steps**: unit yields at the default per-step budget (256
  instructions). Use `--budget 1` for single-instruction stepping.
- **Insns**: rough instruction count to checkpoint (`steps * budget`).
- **Checkpoint**: the deterministic boot stopping point.
  `ProcessExit` is the title calling `sys_process_exit`;
  `FirstRsxWrite` is the title's first PPU write to the RSX
  control register at guest `0xC0000040` (typically inside
  `_cellGcmInitBody`).
- **Cross-runner**: classified verdict against the RPCS3 baseline.

## Matrix

| Serial | Title | Year | Engine | Format | Checkpoint | Steps | Insns | Cross-runner |
|--------|-------|------|--------|--------|------------|------:|------:|--------------|
| NPUA80001 | flOw | 2007 | thatgamecompany | PSN HDD | ProcessExit | 9,942 | ~10K | equivalent (1 byte non-semantic) |
| NPUA80068 | Super Stardust HD | 2007 | Housemarque | PSN HDD | FirstRsxWrite | 14,109,359 | ~3.6B | equivalent (1 byte non-semantic) |
| BCES00664 | WipEout HD Fury | 2009 | Sony Liverpool | Disc ISO | FirstRsxWrite | 20,569 | ~5M | equivalent (974 bytes non-semantic) |

## flOw ProcessExit

flOw's `ProcessExit` checkpoint is a clean self-shutdown. The
title completes its init sequence (GCM context, video-out
query, input init, sysmodule init), then runs its process-wide
destructor chain and calls `sys_process_exit`. It is voluntary
termination, not a CellGov fault or hang.

The trigger is that several video and GCM query functions
(`cellVideoOutGetState`, `cellVideoOutGetResolution`,
`cellGcmAddressToOffset`) return `CELL_OK` but do not yet
populate their out-parameters. The title reads zeros from
those structs, treats the configuration as invalid, and takes
its graceful-exit branch instead of entering its render loop.
