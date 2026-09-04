---
content_id: BCES00664
title: WipEout HD Fury
year: 2009
developer: Sony Liverpool
engine: Studio Liverpool proprietary
distribution: Disc ISO
cell: fw 4.93 x base
checkpoint: FirstRsxWrite
steps: 43055
convergence: Yes
byte_parity: 975 non-semantic
---

WipEout HD Fury converges with RPCS3 at `FirstRsxWrite` (step 43,055,
matching this cell's committed anchor) and every divergent byte
classifies. Both runners are sampled at the same checkpoint: CG at its
`MemError::ReservedWrite` trap on the put-store, RP at the
`CELLGOV_DUMP_PATH_RSX` trigger (first observed
`ctrl->put != initial_put` in the cpu_task loop, per
`bridges/rpcs3-patch/0001-cellgov-checkpoint-dump.patch`).

Both runners mount firmware 4.93: RP's `/dev_flash/` mapping names the
store entry CG composes, so the two sides share one library rather than
two installs of one version. This is the cell the title is gated and
rendered at. It is not the version the title asks for: `PS3_SYSTEM_VER`
names 2.76, and that cell is declared alongside this one.

The RSX1 and RSX2 dumps are byte-identical, so the torn-read noise
floor is zero for this title and the comparison below is a real
pre-checkpoint difference rather than dump tearing.

## Classifier coverage (per-class)

- `HleOpdSlot`: 963 bytes. Function-descriptor slots the two runners
  fill with their own stub addresses. Structural: the slots are located
  from the title's import table, not from the values found in them.
- `SyncPrimitiveId`: 12 bytes. Handle words in `sys_lwmutex_t` /
  `sys_lwcond_t` slots, which each runner numbers from its own
  allocator.

## B1 at `data@0xbfe9c` no longer diverges

Earlier captures of this cell left one byte unclassified, at
`data@0xbfe9c` (vaddr `0x91FE9C`, `+0x0c` of the 32-byte record at
`0x91FE90`): CG held `0x00000000` where RP held `0x01000000`. Both
runners now hold `0x01000000` and the residual is empty.

The RP value is unchanged across every capture of this title held here,
so the CG side is what moved: CG now reaches the firmware-PRX helper
that writes the record, along a path the earlier boot did not take. The
evidence captured while the byte was divergent is in
`docs/dev/bug_investigations/b1_byte_at_0x91FE9C.md`, which describes a
state neither runner is in any more.

## Next step

None outstanding for this cell.
