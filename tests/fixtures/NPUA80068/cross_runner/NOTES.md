---
content_id: NPUA80068
title: Super Stardust HD
year: 2007
developer: Housemarque
engine: Housemarque proprietary
distribution: PSN HDD
checkpoint: FirstRsxWrite
steps: 14352588
convergence: Yes
byte_parity: 620 non-semantic + 104 pending
---

Converges with RPCS3 at FirstRsxWrite (step 14,352,588). 620
bytes classified, 104 bytes in two clusters under investigation.

## Classifier coverage

- `ElfHeader`: 3 bytes. SELF-decrypted header bytes reconstructed
  differently per runner; never read at runtime.
- `HleOpdSlot`: 617 bytes. Primary import-stub table (596) +
  secondary OPD tables at `0x829b10`/`0x829b78` (21; secondary-
  OPD-table populator extension).

## Pending residual

### Cluster 1 -- 4 bytes at `0x81ef64`, `0x81ef7c`, `0x81ef84`

CellGov keeps static EBOOT values; RPCS3 writes:
- `0x81ef64`: `0x00000001` -> `0x00000002`
- `0x81ef7c`: `0x00000000` -> `0x01220000`
- `0x81ef84`: `0x00000001` -> `0x00000004`

CG@30M-step run (2x past FirstRsxWrite, via temp manifest with
`kind="process-exit"`) shows CellGov NEVER writes these. Rules
out title-code-writer-CellGov-hasn't-reached-yet. The writer is
RPCS3-specific machinery.

Suggestive lead: `0x0122` = `CELL_SYSUTIL_SYSTEMPARAM_ID_LICENSE_AREA`
in `tools/rpcs3-src/rpcs3/Emu/Cell/Modules/cellSysutil.h:72`,
and `0x01220000 = (0x0122 << 16)`. Hypothesis: a `cellSysutil`
HLE handler synthesizes the value from the license-area enum
and writes a title-side buffer. Unverified.

Next probe: either a pre-HLE baseline trace patch on RPCS3 (a
`bridges/rpcs3-patch/0003-*.patch` that snapshots watch
addresses at process-start), or a focused RPCS3 source review
of cellSysutil HLE writers to title-side memory.

### Cluster 2 -- 100 bytes at `0x48140..0x481a4`

Scratch buffer, NOT a static-init gap. CG@30M reverts to static
exactly:
- static:    `95f24dab 0b685215 e76ccae7 ...`
- CG@14.35M: `95f24da8 0b685216 e76ccae4 ...` (static XOR 0x03/word)
- CG@30M:    `95f24dab 0b685215 e76ccae7 ...` (matches static)
- RP:        `fc136145 62897efb 8e8de609 ...` (wholly different)

CellGov writes-then-restores; the XOR-0x03 pattern at 14.35M is
a snapshot artifact of catching CG mid-scratch. RPCS3 likely
does the same with different scratch contents at its
FirstRsxWrite moment.

Next probe: per-step memory snapshots of `0x828140..0x8281a4`
on CellGov to identify the scratch-user code path. Likely
non-load-bearing once identified.

## Open questions

1. What RPCS3 mechanism writes `0x01220000` at `0x81ef7c`?
2. What CellGov code uses cluster 2 as scratch?
3. Foundation-anchor disposition: if neither closes, SSHD does
   not enter as a foundation anchor; flOw-only-anchor is the
   contingency. No class lands to absorb either cluster.

## Retracted leads (do not rediscover)

- `ppu_prx_module_info` field-offset match. Refuted: records sit
  outside `lib_ent`/`lib_stub`; 32-byte stride vs struct's 44.
- "Declared SCE descriptor table". Refuted: records 1 and 2 are
  not the same descriptor type; wider .data context shows
  per-title audio/codec data structures, not SCE-ABI.
- "CellGov-init gap, title-code writer CG hasn't reached yet".
  Refuted by the 30M-step run.
- "Cluster-1 bytes causally inert" (post-injection-experiment
  conclusion). Overreach: the diverge-injection result
  (IDENTICAL across 14.29M PpuStateHash records when cluster 1
  is patched to RPCS3 values) proves register-propagation
  absence, not causal inertness. State hash is GPR-side only;
  FPR/VR loads and read-into-store-address chains are invisible
  to it.

## Investigation tools used

- `cellgov_cli run-game --save-state-trace <path>`: writes the
  runtime's per-step PpuStateHash trace to a binary file
  (switches mode `FaultDriven` -> `DeterminismCheck`).
- `cellgov_cli diverge a.state b.state`: localize where two
  CellGov runs first differ.
- Injection methodology: `--patch-byte` to inject RPCS3 values
  into CG memory at boot, then diverge.
- Run-past-checkpoint via temp `--title-manifest` with
  `kind="process-exit"`.
- HLE-trace patch (patch 0002): only sees post-HLE writes,
  blind to pre-HLE. Cluster 1 and 2 writes are pre-HLE, so
  show as zero records in the patched-RPCS3 trace.
