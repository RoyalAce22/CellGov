# lv1_repository_hw_config

A read-only capture of lpar 1's `sys.hw.config` node in lv1's
repository, on the reference console at firmware 4.93. The 4.93 kernel
reads this node at boot. CellGov's hypervisor answers that read from
this capture, through `cellgov_ps3_abi::lv1::repository`.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the values the reads
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
  A second run read the bucket, the node's next pointer, its
  split-order key, its key words and its values again, with the same
  results.
- Only the node's key and values are committed. The raw pages are
  deleted.
- No row of `tests/micro/console_crashes.tsv` covers these addresses.
- No console-identifying value is in this directory.

## The kernel's reads

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives three functions that read the node. Each
one calls the key wrapper at 0x8000000000095A50. The wrapper builds
the four key words from pairs of a text and an index, and calls the
read function, which sets r3 = 1 and makes hypercall 91
[OFW-4.93 lv2_kernel a:0x80000000000922c4-0x8000000000092318]. On
status 0, the wrapper stores r4 through its first argument and r5
through its second
[OFW-4.93 lv2_kernel a:0x8000000000095b28-0x8000000000095b40].

The three callers pass the same four texts, `sys`, `hw`, `config` and
an empty text, each with index 0. Their TOC slots hold the addresses of
the same strings, for example
[OFW-4.93 lv2_kernel a:0x800000000034ac40-0x800000000034ac5c]: `sys`
[OFW-4.93 lv2_kernel a:0x80000000002d93f0], `hw`
[OFW-4.93 lv2_kernel a:0x80000000002d93f8], `config`
[OFW-4.93 lv2_kernel a:0x80000000002d9400-0x80000000002d9404] and the
empty text [OFW-4.93 lv2_kernel a:0x80000000002ee6a8]. The empty text
gives a fourth key word of 0. The TOC base is the doubleword at
[OFW-4.93 lv2_kernel a:0x8000000000003000-0x8000000000003004].

1. The constructor at 0x80000000000A1EA4 builds the read
   [OFW-4.93 lv2_kernel a:0x80000000000a1f3c-0x80000000000a1f68]. It
   reads the node only when its call to 0x8000000000099850 returns 0
   [OFW-4.93 lv2_kernel a:0x80000000000a1f10-0x80000000000a1f1c]. On
   status 0 it keeps v1 at +8 of its object
   [OFW-4.93 lv2_kernel a:0x80000000000a1f6c-0x80000000000a1f7c].
   Later code tests bit 7 of that copy
   [OFW-4.93 lv2_kernel a:0x80000000000a1fd4-0x80000000000a1fe0], or
   clears its bit 18
   [OFW-4.93 lv2_kernel a:0x80000000000a1fa4-0x80000000000a1fbc].
2. The function at 0x8000000000287428 builds the read
   [OFW-4.93 lv2_kernel a:0x800000000028761c-0x800000000028764c]. On
   status 0 it keeps bit 63 and bit 19 of v1
   [OFW-4.93 lv2_kernel a:0x8000000000287764-0x8000000000287774].
3. The function at 0x80000000000B3A60 builds the read
   [OFW-4.93 lv2_kernel a:0x80000000000b3e8c-0x80000000000b3ebc]. On
   status 0 it tests only the sign of v1
   [OFW-4.93 lv2_kernel a:0x80000000000b3ec0-0x80000000000b3ed0].

None of the three functions loads the stack slot that receives v2 after
the read. So the kernel discards v2 at each of the three reads.

The kernel's boot function at 0x8000000000092254 calls the bus search,
then the function at 0x80000000000A1AC8, then the function at
0x8000000000287428
[OFW-4.93 lv2_kernel a:0x8000000000092260-0x8000000000092294]. The
function at 0x80000000000A1AC8 creates the object of read 1 through its
constructor [OFW-4.93 lv2_kernel a:0x80000000000a1c0c-0x80000000000a1c28].

## Where lv1 keeps the node

The README of `tests/micro/lv1_repository_debug_support` gives lv1's
read path and the node layout. The README of
`tests/micro/lv1_repository_flash` gives the bucket rule. In short:

- The hash is the XOR of the four 32-bit halves of n1 XOR n4 and n2
  XOR n3, with the bits of the mask word cleared
  [OFW-4.93 lv1 a:0x2E3820-0x2E38D8].
- The split-order key is the hash with its 32 bits reversed, OR 1
  [OFW-4.93 lv1 a:0x2E3874-0x2E38D8].
- A node holds its next pointer at +48, its split-order key at +56, the
  four key words at +64, v1 at +104 and v2 at +112.

`sys.hw.config` hashes to 0x11061D66, so it is in bucket 6, and its
split-order key is 0x66B86089.

The walk on the reference console:

| Step | Address | Read |
|---|---|---|
| mask word | 0x373D08 | 0xC0000000 |
| partition table pointer | 0x373D18 | 0x609E00 |
| partition table + 24, the segment list | 0x609E18 | 0x60F140 |
| entry of lpar 1 | 0x60F278 | 0x66FA30 |
| lpar 1 partition + 56 | 0x66FA68 | 0x687060, the repository |
| table + 24, n | 0x687080 | 1, so 16 buckets |
| table + 32, segment 0 | 0x687088 | 0x687B80 |
| bucket 6 | 0x687BE8 | 0x687F60, a dummy node with split-order key 0x60000000 |
| `sys.hw.config` node, after the dummy node and 7 nodes with lower split-order keys | 0x689610 | next pointer 0x68B010, split-order key 0x66B86089 |

The walk up to the buckets gave the same values as the walks of
`lv1_repository_flash` and `lv1_repository_bus`. The node holds the four
key words of `sys.hw.config` at +64. Bit 0 of its next pointer is
clear, so the node is not deleted.

## What writes the node

This capture does not trace the lv1 code that creates the node. So it
does not show how the value depends on the unit. Its values are those
of one CECH-2001A.

## Files

| File | Rows | Holds |
|---|---|---|
| `node.tsv` | 1 | `sys.hw.config`: the firmware, the key, the lv1 address of v1, the number of reads, whether they agree, and v1 and v2 |

## The values

`sys.hw.config` holds v1 = 0x4E00FFFF0203BC3C and v2 = 0. lv1 answers
the kernel's read with r3 = 0 and these two values.

On this value, bits 7, 18, 19 and 63 of v1 are clear.
