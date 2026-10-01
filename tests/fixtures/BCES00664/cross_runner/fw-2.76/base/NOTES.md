---
content_id: BCES00664
title: WipEout HD Fury
year: 2009
developer: Sony Liverpool
engine: Studio Liverpool proprietary
distribution: Disc ISO
cell: fw 2.76 x base
checkpoint: FirstRsxWrite
steps: 36883
convergence: Yes
byte_parity: 1020 non-semantic
---

WipEout HD Fury converged with RPCS3 at `FirstRsxWrite` (step 37,131)
and every divergent byte classifies; the cross-runner triple beside this
file is that measurement. The committed anchor now stops earlier, at
step 36,883, with an MFC fault on the first SPU of the title's SPURS
thread group: the group's event connect answers instead of refusing, so
SPURS starts its SPUs, and the kernel's first transfer names an address
built from a thread argument whose high word repeats its low word. With
the arguments laid out as one big-endian doubleword in the preferred
slot the same run reaches the checkpoint at step 37,201.

Both runners mount the same firmware tree: RPCS3's `/dev_flash/`
mapping names the store entry CellGov composes, so the two sides share
one library rather than two installs of one version. `2.76` is the
version this title's own `PARAM.SFO` asks for
(`PS3_SYSTEM_VER = 02.7600`), and the same version its disc carries in
`PS3_UPDATE/PS3UPDAT.PUP`.

## Classifier coverage (per-class)

- `HleOpdSlot`: 1008 bytes. Function-descriptor slots the two runners
  fill with their own stub addresses. Structural: the slots are located
  from the title's import table, not from the values found in them.
- `SyncPrimitiveId`: 12 bytes. Handle words in `sys_lwmutex_t` /
  `sys_lwcond_t` slots, which each runner numbers from its own
  allocator.

## Next step

Lay each SPU thread argument out as a 64-bit value in the preferred
slot when the SPU starts, then re-measure this cell and regenerate the
triple at the checkpoint it reaches. The one byte `fw 4.93 x base` left
unclassified, in a record written by a firmware-PRX helper, has since
closed: CellGov reaches the helper there now, and both runners hold its
constant. That was the CellGov side moving rather than 2.76's helper
differing, so the cross-cell question the byte raised is answered and
this cell never carried it.
