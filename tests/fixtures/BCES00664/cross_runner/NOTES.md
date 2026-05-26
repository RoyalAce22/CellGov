---
content_id: BCES00664
title: WipEout HD Fury
year: 2009
developer: Sony Liverpool
engine: Studio Liverpool proprietary
distribution: Disc ISO
checkpoint: FirstRsxWrite
steps: 43156
convergence: Yes
byte_parity: 1055 non-semantic + 86 pending
---

Reaches the `FirstRsxWrite` checkpoint at step 43,156 with
outcome `RsxWriteCheckpoint`. RPCS3 boots end-to-end through
the same checkpoint and continues to gameplay; the
cross-runner pair still records `convergence: No` because the
operator-declared checkpoint maps to distinct outcomes
(`RsxWriteCheckpoint` vs `Completed`). Byte parity remains
undefined; a future phase that backs the put-target as
ReadWrite (per the manifest's `[rsx] mirror = true` opt-in)
will let both runners reach `Completed` and unblock the byte
walk.

The step count moved 43,066 -> 43,156 between Phase 37 and
Phase 38 because the mmapper handle-table + region-install
work (38B.1 / B.2 / B.3) lets libgcm's `_cellGcmInitBody`
proceed past the unbacked-handout fault. The +90 step delta
is forward progress: WipEout now executes the FIFO command
buffer setup that follows the shared-memory mappings and
issues the put-pointer write at `0xC0000040` (libgcm vaddr
`0x79a0` per Phase 37's Stage A.2), which trips the
FirstRsxWrite checkpoint.

The 3 host invariant breaks during this run are all honest:
the previously-firing
`dispatch.mmapper_map_shared_memory_unbacked` entries are
gone (now backed); the residual breaks are
`dispatch.memory_free_noop` (sys_memory_free against the
bump allocator) plus other unmodeled-no-op handler logs the
boot exercises post-Phase-37. No contaminating fake-success
returns; every CELL_OK comes with real backing.

RPCS3 corpus state (Stage E):
  outcome: Completed
  step: (not recorded in observation)
  checkpoint: FirstRsxWrite (manifest)
  reached: yes -- RPCS3 boots WipEout HD end-to-end through
           the operator-declared FirstRsxWrite point.

## Next step

Two open paths. (1) Enable `[rsx] mirror = true` in
[docs/titles/BCES00664.toml](../../../docs/titles/BCES00664.toml)
so the put-write at `0xC0000040` lands in a ReadWrite shadow
rather than tripping FirstRsxWrite; the boot then proceeds
into the FIFO method-decoder path and eventually reaches the
spin-poll at libgcm vaddr `0x7a08` (the rendering-side wall
documented in Phase 36.7's
`wipeout_commit_fault_step_43066.md`). (2) Model
`NV406E_SET_REFERENCE` so the spin-poll's `ctrl.ref` read
gets a guest-visible write -- a successor named in Phase 37
and Phase 38's Required-successor lists.

## Structural pre-analysis (pending convergence)

Findings below are from static EBOOT analysis plus targeted
reads of the existing non-converged observation pair. They
inform classifier scope; all must be re-verified once a
converged observation pair exists.

### Cluster-3 mechanism confirmed at `data@0xc5008`/`0xc5070` (covered by the secondary-OPD-table populator)

Header-signature scan (`hdr=0x040201xx count=0x000x0000`) finds
two adjacent `0x68`-byte tables, identical layout and spacing to
SSHD's `0x49b10`/`0x49b78`. RPCS3 observation shows table 1
patched with HLE OPDs in the `0x3000_xxxx/0x3001_xxxx` range
SSHD uses; table 2 slots still at static (module not yet
bound, or bound lazily). The populator's `Range<u64>` shape
generalizes without modification.

~1088 bytes at `data@0xc5008..0xc5448` will reclassify as
`HleOpdSlot` once a converged observation pair is captured.

### Slot A: `data@0x61110+0xc` -- RPCS3 overwrites a static debug string

Static bytes spell ASCII `"crt0:p250001"` (a build-version
string the title's CRT0 declares in `.data`). RPCS3 runtime
replaces those 12 bytes with `(0x01000000, 0x00000001,
0x007b5f90)`. CellGov keeps the static string.

Same family signature as SSHD's cluster 1 (CG keeps static, RP
writes). Open whether RPCS3 intentionally repurposes the buffer
or whether this is an HLE side-effect write into title-allocated
memory. The SSHD-side investigation's cellSysutil license-area
lead may apply here too; check after a converged observation
pair exists.

### Big cluster at `data@0xbfe9c..0xc13ca` (~5,422 bytes) -- NOT cluster 3

Audit hypothesis that the big cluster shared cluster 3's
mechanism is wrong. Static-byte content is heterogeneous:

- `0xbfe80` area: IEEE float constants (title data)
- `0xbff00` area: zero-initialized
- `0xc0000` area: random-looking 16-byte-aligned data (crypto / hash?)
- `0xc0c00` area: (id, ptr) pairs (asset / resource manifest)
- `0xc1380` area: (counter, ptr, value) triples

Likely multiple title-specific mechanisms layered in one region.
Decompose into narrower classes once a converged observation pair
exists; do not invent an umbrella class for it.

### CellGov-only sparse slots: non-convergence noise

`data@0x618ac`, `0x61b6c`, `0x629ec`, `0x635d8` show the
opposite asymmetry (CG diverges from static, RP keeps static).
These are likely path-specific writes between the convergent
prefix and CG's fault step. Should disappear after convergence;
if they remain, that's a structural finding worth a new
investigation.

## Retracted leads (do not rediscover)

- "Big cluster at `0xbfe9c` shares cluster-3 mechanism" (audit
  prediction). Refuted: cluster-3 header signature finds tables
  at `0xc5008`/`0xc5070`, not at `0xbfe9c`. Big cluster is a
  different mechanism.
- "Slot A is a `PrxModuleStateField`-shape" (post-SSHD framing).
  Refuted: static bytes are a literal ASCII debug string, not
  any kind of `ppu_prx_module_info` instance.

## Classifier scope implications

The four-variant `DivergenceClass` enum (`ElfHeader`,
`SysProcParam`, `HleOpdSlot`, `Unclassified`) is current and
intended. WipEout adds no new variants; cluster-3-mechanism
bytes classify as `HleOpdSlot` via the secondary-OPD-table
populator, everything else stays
`Unclassified` pending the converged observation pair and a
mechanism walk.

For any new title fixture: re-run the SSHD cluster-3 signature
scan (`cellgov_ppu::loader::find_secondary_opd_tables`) against
the new EBOOT. Two adjacent tables means the mechanism is
present and the secondary-OPD-table populator covers it; zero
or one means look further.
