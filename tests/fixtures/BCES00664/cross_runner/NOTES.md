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
byte_parity: 975 non-semantic + 1 pending
---

Reaches `FirstRsxWrite` at step 43,156 deterministically across
two runs. Both runners are sampled at the same checkpoint: CG at
its `MemError::ReservedWrite` trap on the put-store, RP at the
`CELLGOV_DUMP_PATH_RSX` trigger (first observed
`ctrl->put != initial_put` in the cpu_task loop, per
`bridges/rpcs3-patch/0001-cellgov-checkpoint-dump.patch`).

## The 1 pending byte: B1 at `data@0xbfe9c+1`

CG=`0x00`, RP=`0x01`. Surrounding 32-byte block:

```
+0xbfe90  CG: 0000000a ffffffff 00000000 00000000
+0xbfea0  CG: 00000001 00000000 00000000 00000001
```

Not a `sys_lwmutex_t` (the `0xffffffff` sentinel sits at +0x04,
not +0x00; no valid lwmutex attribute at +0x08), so the
`SyncPrimitiveId` populator correctly leaves it Unclassified.
The divergent byte is the high byte of a u32 at +0x0c of this
block; RP stores `0x01000000`, CG keeps `0x00000000`. Likely a
title-side boolean-as-u32 init flag the title sets at boot;
CG's pre-put-store execution path does not reach the writer.

Phase 39 follow-up: identify the writer (run with
`CELLGOV_HLE_WATCH=0x91fe9c:4` + `cellgov_cli rpcs3-attribute
--addr 0x91fe9c`) and decide whether to model the upstream
path or classify the byte.
