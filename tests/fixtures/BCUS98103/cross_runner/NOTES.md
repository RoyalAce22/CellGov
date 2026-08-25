---
content_id: BCUS98103
title: Uncharted: Drake's Fortune
year: 2007
developer: Naughty Dog
engine: Naughty Dog proprietary
distribution: Disc ISO
checkpoint: FirstRsxWrite
steps: 7119
convergence: Yes
byte_parity: 666 non-semantic + 57 pending
---

Reaches `FirstRsxWrite` at step 7,119 deterministically across
two runs (`bench-boot --title uncharted` matches the committed
anchor). Both runners are sampled at the same checkpoint: CG at
its `MemError::ReservedWrite` trap on the put-store, RP at the
`CELLGOV_DUMP_PATH_RSX` trigger. The RSX1 and RSX2 dumps are
byte-identical, so the torn-read noise floor is zero for this
title and every pending byte below is a real pre-checkpoint
difference, not dump tearing.

Boot shape: 164 of 172 imports resolve to firmware OPDs; the 8
`libvdec` imports sit on the unresolved-import trampoline because
the firmware `libvdec.sprx` is pruned (its own `libdivx311dec`
import has no provider). One host invariant break:
`sys_spu_initialize` announces limits (`max_usable_spu=6`) that
the kernel model does not persist. 270 `sys_tty_write` captures
are dropped because the title logs from stack buffers above the
1 GiB main-memory bound the TTY capture checks against; the
writes themselves complete.

The EBOOT carries four loadable segments; the title's engine data
lives in the second RW segment (`data_hi` at `0x100a0000`), which
is where every pending byte sits.

## Classifier coverage (per-class)

- `HleOpdSlot`: 657 bytes. Firmware-OPD slots in `data@0x780000`
  whose pointer values differ only by where each runner loaded the
  same firmware modules.
- `SyncPrimitiveId`: 9 bytes. Three `sys_lwmutex_t` records at
  `data_hi@0xe2250`, `0xe2270`, `0xe22b0` whose `sleep_queue`
  field holds each runner's kernel id (CG `0x27..0x29`, RP
  `0x95001e00..0x95002000`).

## Unclassified residual (57 bytes, 22 runs)

- `data_hi@0x150..0x27f` (14 runs, 14 bytes): twelve 32-bit words
  spread over three parallel 0x8c-byte records (floats, two
  pointers into `data_hi`, a count of 3). On RP each word holds a
  small sequential id (`0x46`, then `0x47..0x4b`, `0x4c..0x4e`,
  `0x4f..0x51`; `0x46` recurs in every record); on CG all twelve
  are zero. Shape: an out-parameter a kernel or firmware call
  fills on RP and leaves untouched on CG. Not file descriptors --
  CG's fd allocator is base-3, never-recycling, and would have
  produced small non-zero values too. Successor: an RP HLE-trace
  run with `CELLGOV_HLE_WATCH=0x100a0150:0x20` to name the writer,
  then the matching CG-side handler.
- `data_hi@0x2ab4..0x2abf` (2 runs, 10 bytes): a
  `{ptr, count, ptr}` triple. RP `{0x10022be0, 0x206,
  0x10022a68}` (both pointers into the RX segment at
  `0x10000000`); CG `{0x1001ba70, 0xffffffff, 0x1001ba70}` -- an
  empty range with a -1 count. Hypothesis: a cursor over a table
  in `code_hi` that CG's boot never advances. Successor: same
  HLE-trace attribution, watch `0x100a2ab0:0x10`.
- `data_hi@0x125cd` (1 byte): a flag byte, CG `0x80` vs RP `0x10`,
  inside a word that reads `0x00800000` vs `0x00100000`.
  Successor: attribution as above.
- `data_hi@0xe21f4` (1 byte): a zero-vs-one word directly after
  two `data_hi` pointers. Successor: attribution as above.
- `data_hi@0xe22d3` (1 byte): a small count adjacent to the three
  classified lwmutexes, CG `0x4f` vs RP `0x4c`. Successor:
  attribution as above.
- `data_hi@0xe2350..0xe2383` (3 runs, 30 bytes): a header word
  (CG `0x04020100` vs RP `0x44420100`) followed by seven pointer
  slots. RP holds `0x4b153e90` / `0x4b163e90`, addresses in a
  user-memory allocation; CG holds `0x10182393` / `0x10182394`,
  which point 0x40 bytes past the slots themselves and differ by
  one byte where RP's differ by 0x10000. Two slots RP fills are
  zero on CG. Shape: the title records the result of a memory
  allocation it made through a syscall that returned differently
  on CG. Successor: attribute the allocation (`sys_memory_allocate`
  / `sys_mmapper_*` returns) on both runners; the CG value is the
  more suspicious of the two.

Every cluster is inert at `FirstRsxWrite`: the step counts agree
exactly and the divergent words are not read on the path to the
checkpoint on either runner.

## Next step

Attribute the six clusters with one RP HLE-trace run carrying all
the watch addresses above, then decide per cluster between a CG
fix (the allocation-result and out-parameter shapes) and a new
structurally-grounded `DivergenceClass` (the id-namespace shapes).
