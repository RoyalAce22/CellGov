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
| NPUA80001 | flOw | 2007 | thatgamecompany | PSN HDD | Fault (lwmutex routing gap) | 78,199 | ~20M | not available (see note below) |
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

flOw boots through CRT0, video-out probe, GCM init, PSSG
renderer init, and the SPURS PPU-side surface init. Its
manifest enables `[rsx] mirror = true`, so the title's
put-pointer store at `0xC0000040` lands in the FIFO cursor
instead of faulting and the commit-boundary FIFO advance pass
dispatches the queued NV4097 / NV406E commands. The GPU
semaphore writebacks emitted by the advance pass satisfy PSSG's
init-completion poll, and the title runs through the full
SPURS PPU surface (CellSpurs control block populated, workload
registry honoring AddWorkload calls, ready-count and contention
controls live, info snapshot accurate).

The current stopping point is a fault at PPU step 78,199. A
helper PPU thread executes a C++ virtual-call dispatcher
(`lwz r9, 0(r3); lwz r11, 0(r9); lwz r0, 0(r11); mtctr r0;
bctrl`) where `*r3` is zero, so `bctrl` jumps to PC=0. The
fault is downstream of an LV2 sync-primitive routing gap: the
HLE wrappers for `sys_lwmutex_lock` / `unlock` / `trylock` /
`destroy` (NIDs 0x1573dc3f, 0x1bc200f4, 0xaeb78725, 0xc3476d0c)
are listed as `Stateful` and `impl` in the NID database but
have no match arms in the runtime dispatcher; calls silently
return CELL_OK without serializing access. Threads contend on
a lock that does not actually lock, leaving a C++ object's
vtable un-initialised when the helper performs its virtual
call. The mini-trace at fault confirms the loop: alternating
`Sc` instructions at the lwmutex_lock / unlock / ppu_thread_get_id
HLE thunks plus a `SyscallResponseTable::insert` displacement
on a `CondWakeReacquire` for mutex `0x4000001A`.

Cross-runner observation against RPCS3 is unavailable until
flOw reaches a mutually-deterministic stopping point past the
sync-primitive routing gap. See
`docs/dev/phases/baselines/phase_26_baselines.md` for the
detailed disassembly and routing-gap source-level evidence.
