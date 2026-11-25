# lv1_storage_placeholder_nodes

A read-only capture, in lv1 on the reference console, of the storage bus
nodes of lpar 1 that the capture `lv1_storage_registration` did not
read. With that capture, it gives the two values of each of the 290
`bus.4` nodes of lpar 1's repository.

CellGov's hypervisor models these nodes through
`cellgov_ps3_abi::lv1::storage_registration`. A test of that module
holds the model to `node.tsv` of this directory.

No probe runs, so this directory has no manifest and no runner frame.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-18, `runner_ps3 peek --space lv1 --width 8` read the
  doublewords through webMAN's `peek.lv1` command, after the console had
  booted to the XMB. It read each page twice, and the two reads of every
  word agree. It wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- Every word that the capture read is lv1 heap, in the span 0x68B950
  to 0x694307. No address is MMIO. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range.
- The walk of `tests/micro/lv1_repository_list` gives the address of
  each node. The capture took the 154 `bus.4` nodes of that walk that
  `lv1_storage_registration` holds no value for. For each node, it read
  the four key words at +64 to +88, v1 at +104 and v2 at +112. The four
  key words of every node agree with the key that `list.tsv` of the walk
  gives for the node's address.
- `node.tsv` keeps each node's key and its two values, in the shape of
  `node.tsv` of `lv1_storage_registration`. Its `address` column is the
  lv1 address of v1. The raw output of the peek is deleted.
- No console-identifying value is in this directory.

## The values

- `bus.4.id` holds (4, 0) and `bus.4.type` holds (5, 0).
- For devices 5 and 6, which lv1 does not report, `id` holds (the id,
  0) and `intr.0` holds (2, 0). Each other node holds the one-time
  initializer's placeholder: `region.<i>.id` holds (i, 0xDEADBEEF), and
  each other field holds (0, 0xDEADBEEF).
- For devices 0 to 3, each region above the first region at or above
  `n_regs` holds the same placeholders. Device 4 has seven regions, so
  `lv1_storage_registration` already holds its eighth.

## Files

| File | Rows | Holds |
|---|---|---|
| `node.tsv` | 154 | the nodes: key, the lv1 address of v1, the reads, whether they agree, v1 and v2 |
