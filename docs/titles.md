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
| NPUA80001 | flOw | 2007 | thatgamecompany | PSN HDD | MaxSteps (4B) | 15,625,000 | 4B | not available (see note below) |
| NPUA80068 | Super Stardust HD | 2007 | Housemarque | PSN HDD | FirstRsxWrite | 14,341,441 | ~3.7B | code:0x35 ELF-header byte (non-semantic) |
| BCES00664 | WipEout HD Fury | 2009 | Sony Liverpool | Disc ISO | MaxSteps (1B) | 3,906,250 | 1B | not available (see note below) |

## WipEout HD Fury cross-runner note

The earlier CellGov release tripped `FirstRsxWrite` on WipEout at
step 20,569 and reported a 974-byte non-semantic divergence against
RPCS3 at that checkpoint. That checkpoint turned out to be spurious:
a PPU decoder bug (`rldimi` mis-decoded as `rldicl`) was corrupting
the upper half of 64-bit pointers during init, producing a stray
store to the RSX MMIO register at `0xC0000040` that tripped the
checkpoint inside CellGov's own harness.

With the decoder fix in place, WipEout's init runs past that point.
Its boot was not observed to reach a natural stopping event within
a 1B-instruction window (3,906,250 scheduler steps with budget 256).
A new mutually-reachable checkpoint needs to be chosen before
WipEout's cross-runner entry can be restored. See
`tests/fixtures/BCES00664_cross_runner/compare_report.txt` for
details.

## flOw cross-runner note

An earlier CellGov release tripped `ProcessExit` on flOw at step
10,872 with code `0x80010005` (CELL_ESRCH-class). That exit was
the title's PSSG renderer init bailing because
`sys_ppu_thread_create_ex` returned `CELL_EFAULT` (a sysPrxForUser
HLE wrapper read r4 as a struct pointer instead of as the entry
OPD address per the SDK ABI, and the LV2 host read PS3 OPDs as
16-byte u64-pair structs instead of 8-byte u32-pair structs).

With both bugs fixed, flOw's PSSG init completes and the title
advances into its main loop. Its boot now runs past the previous
exit point and is not observed to reach a natural stopping event
within a 4B-instruction window (15,625,000 scheduler steps with
budget 256). The current steady-state is a polling loop on
`cellGcmGetControlRegister` + `cellGcmAddressToOffset` waiting
for the RSX FIFO get-pointer to advance -- a separate
RSX/vblank gap that needs its own coverage before flOw can
reach a mutually-reachable checkpoint with RPCS3 again.
