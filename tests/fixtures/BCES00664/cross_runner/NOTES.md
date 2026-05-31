---
content_id: BCES00664
title: WipEout HD Fury
year: 2009
developer: Sony Liverpool
engine: Studio Liverpool proprietary
distribution: Disc ISO
checkpoint: FirstRsxWrite
steps: 43083
convergence: Yes
byte_parity: 975 non-semantic + 1 pending
---

Reaches `FirstRsxWrite` at step 43,083 deterministically across
two runs. Both runners are sampled at the same checkpoint: CG at
its `MemError::ReservedWrite` trap on the put-store, RP at the
`CELLGOV_DUMP_PATH_RSX` trigger (first observed
`ctrl->put != initial_put` in the cpu_task loop, per
`bridges/rpcs3-patch/0001-cellgov-checkpoint-dump.patch`).

## The 1 pending byte: B1 at `data@0xbfe9c+1`

CG=`0x00`, RP=`0x01`. Vaddr `0x91FE9C` (`+0x0c` of the
32-byte record at `0x91FE90`). Surrounding 32-byte block:

```
+0xbfe90  CG: 0000000a ffffffff 00000000 00000000
+0xbfea0  CG: 00000001 00000000 00000000 00000001
```

Not a `sys_lwmutex_t` (`0xffffffff` sentinel sits at +0x04,
not +0x00; no valid lwmutex attribute at +0x08), so the
`SyncPrimitiveId` populator correctly skips it.

### Writer (RP side)

Single guest PPC `std` covering `ea = 0x0091FE98` width 8
with value `0x0000000001000000`. In BE order this places
`+0x08 = 0x00000000` and `+0x0c = 0x01000000` in one
store; there is no separate writer for `+0x0c`.

Block containing the store (firmware-PRX text, reachable
via `cellgov_cli disasm` against the loaded firmware
image at PC `0x01664EE4`):

```
0x01664EE4  or    r9, r3, r3              ; r9 = r3 (out_ptr arg)
0x01664EE8  or    r11, r13, r13           ; r11 = r13 (TOC)
0x01664EEC  addi  r11, r11, -28720        ; r11 = TOC - 0x7030
0x01664EF0  addi  r3, r0, 0               ; r3 = 0 (return value)
0x01664EF4  rldicl r11, r11, 0, ...       ; mask
0x01664EF8  ld    r0, 0(r11)              ; r0 = *(TOC + offset)
0x01664EFC  std   r0, 0(r9)               ; *out_ptr = r0
0x01664F00  bclr                          ; return
```

Shape: a "write-TOC-constant-through-out-ptr" helper. The
stored value `0x0000000001000000` is a firmware-PRX TOC
constant -- runner-independent, identical on any host
that loads the same firmware.

### Non-writer (CG side)

CG's commit pipeline writes the same 8-byte span at the
same `ea = 0x0091FE98` with value zero. CG does not retire
the firmware helper at PC `0x01664EE4` along its
pre-checkpoint trajectory; it reaches the destination EA
via a different call graph that does not invoke the
helper.

Verifiable from shipped tooling:
`target/release/cellgov_cli.exe bench-boot --title wipeout
--checkpoint pc=0x6516F0 --max-steps 100000000` halts at
step 43,156 / Fault without firing the PC check at
`0x6516F0` (the title call site preceding the path that
on RP reaches the helper).

### Behavioral status pre-checkpoint

Title-internal: the containing-record address is never
read or written via any HLE call along either runner's
pre-checkpoint trajectory; the record stays inside the
title's data segment.

Inert at FirstRsxWrite: CG reaches FirstRsxWrite at
step 43,156, `host_invariant_breaks=3`, bit-identical
across two `bench-boot --title wipeout` runs. CG and RP
read divergent values at `+0x0c` along their respective
pre-checkpoint trajectories with no resulting step-count
divergence.

### Classification

B1 is bucketed as `Unclassified` -> `+1 pending` because
no `DivergenceClass` covers "shared destination, divergent
call graph, title-internal, inert at this checkpoint."
The skipped firmware helper writes a runner-independent
constant; the byte itself is not a non-semantic field of
the type `ElfHeader` / `SysProcParam` / `HleOpdSlot` /
`SyncPrimitiveId` claim.
