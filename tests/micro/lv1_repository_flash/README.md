# lv1_repository_flash

A read-only capture of lpar 1's `sys.flash.ext` and `sys.flash.boot`
repository nodes on the reference console, at firmware 4.93. The 4.93
kernel reads `sys.flash.ext` at boot, in its boot SPE create function.
CellGov's hypervisor answers that read from this capture, through
`cellgov_ps3_abi::lv1::repository`.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the values the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A,
  a retail (CEX) unit, on 4.93 with custom firmware (EvilNAT 4.93 PEX,
  Cobra 8.5) and webMAN MOD 1.47.48. It is not a debug or tool unit.
- The capture used `cellgov ps3 peek --space lv1` only, through the
  runner's console queue, after the console had booted to the XMB, on
  2026-11-12. The command reads webMAN's `peek.lv1` pages. The capture
  wrote nothing to lv1 or lv2, used no poke and installed nothing on the
  console.
- The command read each page twice. The two reads of every word agree.
- Only the two nodes' keys and values are committed. The raw pages are
  deleted.
- No console-identifying value is in this directory.

## The kernel's read

The read is one leaf function in the shipped 4.93 `lv2_kernel.self`
[OFW-4.93 lv2_kernel a:0x8000000000050ddc-0x8000000000050e48]:

- It builds lpar 1 and the key words `sys`, `flash`, `ext`, 0 in r3 to
  r7, and makes hypercall 91
  [OFW-4.93 lv2_kernel a:0x8000000000050e14].
- It keeps only r4. It returns 1 when r3 is 0 and bit 1 (mask 2) of r4
  is clear. It returns 0 for any other answer
  [OFW-4.93 lv2_kernel a:0x8000000000050e18-0x8000000000050e48].

The boot SPE create function calls it after hypercall 64
[OFW-4.93 lv2_kernel a:0x800000000004f23c]. The return value selects
the limit that the function passes to hypercall 138 in r4
[OFW-4.93 lv2_kernel a:0x800000000004f240-0x800000000004f278]:

- On a return of 1, the function reads `sys.flash.boot` in the leaf
  function at 0x8000000000050D5C
  [OFW-4.93 lv2_kernel a:0x800000000004f24c]. That reader returns 1 only
  when r3 is 0 and r4 is 1
  [OFW-4.93 lv2_kernel a:0x8000000000050d98-0x8000000000050dd8]. On its
  return of 1, the limit is 0.
- On any other path, the limit is 120 times the value of the function
  at 0x8000000000014148
  [OFW-4.93 lv2_kernel a:0x800000000004f260-0x800000000004f26c].

## Where lv1 keeps the node

The README of `tests/micro/lv1_repository_debug_support` gives lv1's
read path, the hash, the node layout and the walk to lpar 1's
repository. This capture adds the bucket rule:

- The hash table is the repository's address plus 8
  [OFW-4.93 lv1 a:0x2E3D20].
- The bucket is the hash AND (2 to the power (n + 3)) minus 1. n is
  the word at table + 24 [OFW-4.93 lv1 a:0x2C9774-0x2C9798].
- Buckets 0 to 15 are in segment 0, whose address is the doubleword at
  table + 32. Bucket b is the doubleword at segment + 56 + 8b
  [OFW-4.93 lv1 a:0x2C97A0-0x2C97B8].
- The walk goes from the bucket's dummy node down the list. It stops at
  the first node whose split-order key is larger than the key's. It
  answers the first node whose split-order key and four key words are
  equal to the key's [OFW-4.93 lv1 a:0x2E39A0-0x2E39F8].
- A node whose next pointer has bit 0 set is deleted
  [OFW-4.93 lv1 a:0x2E39BC-0x2E39C8].
- A key with no node answers r3 = 0x0000001EFFFFFFFA
  [OFW-4.93 lv1 a:0x2E3E9C-0x2E3EB8].

The walk on the reference console:

| Step | Address | Read |
|---|---|---|
| mask word | 0x373D08 | 0xC0000000 |
| partition table pointer | 0x373D18 | 0x609E00 |
| partition table + 24, the segment list | 0x609E18 | 0x60F140 |
| segment 0 | 0x60F140 | 0x609F60 |
| segment 0 + 48, the entry size | 0x609F90 | 8 |
| segment 0 + 72, its entry array | 0x609FA8 | 0x60F270 |
| entry of lpar 1 | 0x60F278 | 0x66FA30 |
| lpar 1 partition + 56 | 0x66FA68 | 0x687060, the repository |
| table + 24, n | 0x687080 | 1, so 16 buckets |
| table + 32, segment 0 | 0x687088 | 0x687B80 |
| bucket 3 | 0x687BD0 | 0x687D80, a dummy node with split-order key 0xC0000000 |
| `sys.flash.ext` node, after 12 nodes with lower split-order keys | 0x688D10 | next pointer 0x68A290, split-order key 0xCE66B619 |
| bucket 7 | 0x687BF0 | 0x687CE0, a dummy node with split-order key 0xE0000000 |
| `sys.flash.boot` node, after 2 nodes with lower split-order keys | 0x688C90 | next pointer 0x68B410, split-order key 0xE0BE5EF9 |

The walk up to the repository gave the same values as the walk of
`lv1_repository_debug_support` on 2026-11-11.

`sys.flash.ext` hashes to 0x186D6673, so it is in bucket 3, and its
split-order key is 0xCE66B619. `sys.flash.boot` hashes to 0x1F7A7D07,
so it is in bucket 7, and its split-order key is 0xE0BE5EF9. Each node
holds those key words at +64 and the key's split-order key at +56. Bit 0
of each node's next pointer is clear, so neither node is deleted.

## What writes the nodes

No lv1 code that creates `sys.flash.ext` is located. The processes in
lv1's process archive read `sys.flash.boot` and `sys.flash.fmt`, for
example [OFW-4.93 lv1 a:0x45FC50-0x45FD2C]. No code in lv1 builds the
`ext` key word with an immediate. So this capture does not show how the
value depends on the unit. Its values are those of one CECH-2001A.

## Files

| File | Rows | Holds |
|---|---|---|
| `node.tsv` | 1 | `sys.flash.ext`: the firmware, the key, the lv1 address of v1, the number of reads, whether they agree, and v1 and v2 |
| `unread_node.tsv` | 1 | `sys.flash.boot`, in the same columns |

## The values

- `sys.flash.ext` holds v1 = 0xFE and v2 = 0. lv1 answers the kernel's
  read with r3 = 0 and these two values.
- `sys.flash.boot` holds v1 = 0x190 and v2 = 0.

Bit 1 of 0xFE is set, so the kernel's `sys.flash.ext` reader returns 0.
The 4.93 boot on this console therefore does not read `sys.flash.boot`.
It passes 120 times the value of the function at 0x8000000000014148 to
hypercall 138. CellGov serves no `sys.flash.boot` node, and the file
`unread_node.tsv` records the node for a later boot that reads it.
