---
content_id: BCES00664
title: WipEout HD Fury
year: 2009
developer: Sony Liverpool
engine: Studio Liverpool proprietary
distribution: Disc ISO
checkpoint: FirstRsxWrite
steps: 45689
convergence: No (outcome: Fault vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at FirstRsxWrite: CellGov faults
at step 45,689 before RPCS3 completes at 45,697. Byte parity
undefined until convergence.

## Next step

Resolve the CellGov fault OR pick a PC-based checkpoint reachable
on both runners before the fault, then regenerate the fixture
without `--allow-divergence`.

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
