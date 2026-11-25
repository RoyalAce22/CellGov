# lv1_repository_list

A read-only walk of the whole list of lpar 1's repository in lv1, on
the reference console at firmware 4.93, and the values of the nodes
that the 4.93 kernel reads at boot and that no other capture gives.
CellGov's hypervisor answers those reads from this capture, through
`cellgov_ps3_abi::lv1::repository`.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the values the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A,
  a retail (CEX) unit, on 4.93 with custom firmware (EvilNAT 4.93 PEX,
  Cobra 8.5) and webMAN MOD 1.47.48. It is not a debug or tool unit.
- The capture used `cellgov ps3 peek --space lv1 --width 8` only,
  through the runner's console queue, after the console had booted to
  the XMB, on 2026-11-16. The command reads webMAN's `peek.lv1` pages.
  The capture wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- The command read each page twice. The two reads of every word agree.
- The walk went from the dummy node of bucket 0 to the list's last
  entry. For each entry it read the next pointer, the split-order key
  and the four key words. It read no value of a node in the walk.
- A second run read the bucket of each served node, the node's next
  pointer, its split-order key, its key words and its two values. The
  words that the walk also read have the same values.
- `list.tsv` keeps the key of each node of the list. `node.tsv` keeps
  the values of the served nodes only. The raw pages are deleted.
- No row of `tests/micro/console_crashes.tsv` covers these addresses.
- No console-identifying value is in this directory.

## Where lv1 keeps the nodes

The README of `tests/micro/lv1_repository_debug_support` gives lv1's
read path and the node layout. The README of
`tests/micro/lv1_repository_flash` gives the bucket rule. In short:

- The hash is the XOR of the four 32-bit halves of n1 XOR n4 and n2
  XOR n3, with the bits of the mask word cleared
  [OFW-4.93 lv1 a:0x2E3820-0x2E38D8].
- The split-order key is the hash with its 32 bits reversed, OR 1
  [OFW-4.93 lv1 a:0x2E3874-0x2E38D8].
- A node holds its next pointer at +48, its split-order key in the high
  word at +56, the four key words at +64, v1 at +104 and v2 at +112.
- The walk tells two nodes with one hash apart by the four key words
  [OFW-4.93 lv1 a:0x2E39D4-0x2E39F8].

The walk up to the buckets gave the same values as the walks of
`lv1_repository_flash`, `lv1_repository_bus` and
`lv1_repository_hw_config`:

| Step | Address | Read |
|---|---|---|
| mask word | 0x373D08 | 0xC0000000 |
| partition table pointer | 0x373D18 | 0x609E00 |
| lpar 1 partition + 56 | 0x66FA68 | 0x687060, the repository |
| table + 24, n | 0x687080 | 1, so 16 buckets |
| table + 32, segment 0 | 0x687088 | 0x687B80 |
| bucket 0 | 0x687BB8 | 0x6881A0, the first dummy node |

## The whole list

The list held 401 entries in ascending split-order key:

- 16 dummy nodes, one for each bucket. Each one holds the bit-reversed
  bucket number as its split-order key, which is even.
- 384 nodes, each with an odd split-order key. The split-order key of
  each one is the key that the hash of its four key words gives.
- The last entry, with the split-order key 0xFFFFFFFF and a next
  pointer of 0.

Bit 0 of each next pointer is clear, so no entry is deleted. The 384
nodes have 384 different keys. Bus 4 has 290 of them.

`list.tsv` keeps the 384 nodes in the order of the walk. A dummy node
is 0x50 bytes long, so the words at its +64 are not key words, and
`list.tsv` keeps no dummy node and not the last entry.

So the walk covered the whole list. A key of lpar 1 that `list.tsv`
does not hold names no node on the reference console, on the day of
the walk.

## Served nodes

### `ss.param.update.status`

The kernel's reader is one leaf function in the shipped 4.93
`lv2_kernel.self`, decrypted offline and read with `cellgov dev disasm`
[OFW-4.93 lv2_kernel a:0x8000000000050ab4-0x8000000000050b50]:

- It builds lpar 1 and the key words `ss`, `param`, `update` and
  `status` with immediates
  [OFW-4.93 lv2_kernel a:0x8000000000050ab4-0x8000000000050b08].
- When its first argument, an out pointer, is 0, it returns 0x80010002
  and makes no hypercall
  [OFW-4.93 lv2_kernel a:0x8000000000050ad0-0x8000000000050b20].
- Otherwise it makes hypercall 91
  [OFW-4.93 lv2_kernel a:0x8000000000050b24]. On status 0 it stores
  the low byte of r4 through the out pointer and returns 0. On any
  other status it returns 0x8001002B
  [OFW-4.93 lv2_kernel a:0x8000000000050b28-0x8000000000050b50].

The reader does not store r5. So the kernel discards v2 at each read
of the node.

Six functions call the reader. Each one compares the stored byte with
0xFF:

1. The caller at 0x800000000005A700
   [OFW-4.93 lv2_kernel a:0x800000000005a6fc-0x800000000005a718].
2. The constructor at 0x80000000000A1EA4, which the 4.93 boot runs
   right after its `sys.hw.config` read
   [OFW-4.93 lv2_kernel a:0x80000000000a1f80-0x80000000000a1fa0]. On a
   byte of 0xFF it goes on to the test of bit 7 of its `sys.hw.config`
   copy [OFW-4.93 lv2_kernel a:0x80000000000a1fd4-0x80000000000a1fe0].
   On another byte it first clears bit 18 of that copy
   [OFW-4.93 lv2_kernel a:0x80000000000a1fa4-0x80000000000a1fbc].
3. The caller at 0x80000000000E3944
   [OFW-4.93 lv2_kernel a:0x80000000000e393c-0x80000000000e395c].
4. The caller at 0x80000000000E7C6C
   [OFW-4.93 lv2_kernel a:0x80000000000e7c68-0x80000000000e7c80].
5. The caller at 0x800000000026CE5C. It compares the byte with 1 and
   then with 0xFF
   [OFW-4.93 lv2_kernel a:0x800000000026ce58-0x800000000026ce78].
6. The caller at 0x800000000026D024, which reads the node again and
   joins the same two compares
   [OFW-4.93 lv2_kernel a:0x800000000026d020-0x800000000026d044].

The node hashes to 0x29007774, so it is in bucket 4, and its
split-order key is 0x2EEE0095:

| Step | Address | Read |
|---|---|---|
| bucket 4 | 0x687BD8 | 0x688100, a dummy node with split-order key 0x20000000 |
| `ss.param.update.status` node, after the dummy node and 16 nodes with lower split-order keys | 0x763DF0 | next pointer 0x690690, split-order key 0x2EEE0095 |

The node holds the four key words of `ss.param.update.status` at +64.
Bit 0 of its next pointer is clear, so the node is not deleted.

This capture does not trace the lv1 code that creates the node. So it
does not show how the value depends on the unit or on its update
history. Its values are those of one CECH-2001A.

`ss.param.update.status` holds v1 = 0xFF and v2 = 0. lv1 answers the
kernel's read with r3 = 0 and these two values
[OFW-4.93 lv1 a:0x2E53AC-0x2E53B8]. The reader stores the byte 0xFF.

## Files

| File | Rows | Holds |
|---|---|---|
| `list.tsv` | 384 | each node of the list, in the order of the walk: the firmware, the key, the lv1 address of the node, its split-order key, the number of reads, and whether they agree |
| `node.tsv` | 1 | each served node: the firmware, the key, the lv1 address of v1, the number of reads, whether they agree, and v1 and v2 |
