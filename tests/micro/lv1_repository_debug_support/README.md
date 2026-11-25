# lv1_repository_debug_support

A read-only capture of lpar 1's `ss.param.debug.support` repository
node on the reference console, at firmware 4.93. The 4.93 kernel reads
this node at boot. CellGov's hypervisor answers that read from this
capture, through `cellgov_ps3_abi::lv1::repository`.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the values the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A,
  a retail (CEX) unit, on 4.93 with custom firmware (EvilNAT 4.93 PEX,
  Cobra 8.5) and webMAN MOD 1.47.48. It is not a debug or tool unit.
- webMAN's `peek.lv1` and `peek.lv2` HTTP commands each return 0x200
  bytes of memory at an address. The capture used those two commands
  only, after the console had booted to the XMB, on 2026-11-11. It
  wrote nothing to lv1 or lv2, used no poke and installed nothing on
  the console.
- The lv1 walk below took thirteen `peek.lv1` reads. The page that
  holds the node was read twice more. The two reads are identical.
- The kernel page that holds the kernel's copy was read twice. The two
  reads are identical.
- Only the node's key and values and the kernel's copy are committed.
  The raw HTTP pages are deleted.
- No console-identifying value is in this directory.

## The kernel's read

The read is one leaf function in the shipped 4.93 `lv2_kernel.self`
[OFW-4.93 lv2_kernel a:0x8000000000050a08-0x8000000000050a8c]:

- It builds lpar 1 and the key words `ss`, `param`, `debug`,
  `support` in r3 to r7, and makes hypercall 91
  [OFW-4.93 lv2_kernel a:0x8000000000050a50].
- When the low word of r3 is 0, it stores r4 and r5 through two TOC
  slots [OFW-4.93 lv2_kernel a:0x8000000000050a70-0x8000000000050a80].
  On any other status it stores nothing.
- The slots point at two doublewords
  [OFW-4.93 lv2_kernel a:0x80000000003492c8-0x80000000003492d4]. The
  image holds all-ones in both
  [OFW-4.93 lv2_kernel a:0x8000000000354d98-0x8000000000354da4].
- Its only caller returns 0 whatever the status is
  [OFW-4.93 lv2_kernel a:0x8000000000050a90-0x8000000000050ab0].

So an answer of no entry and an answer of all-ones leave the same
kernel memory. The kernel's copy alone cannot tell the two apart. The
lv1 walk can.

## Where lv1 keeps the node

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the read path:

- Hypercall 91 is a memory call. Its handler
  [OFW-4.93 lv1 a:0x2E52C4] finds the partition that r3 names in the
  partition table
  [OFW-4.93 lv1 a:0x2E53F4-0x2E54B8]. The table's pointer is the
  doubleword that the TOC slot [OFW-4.93 lv1 a:0x358228] locates.
- The partition's repository is the doubleword at partition + 56
  [OFW-4.93 lv1 a:0x2E5388]. The handler looks the key up in it
  [OFW-4.93 lv1 a:0x2E53A0] and returns the status, v1 and v2
  [OFW-4.93 lv1 a:0x2E53AC-0x2E53B8].
- The repository is a split-ordered hash list. The hash mixes the four
  key words and clears the bits of a mask word
  [OFW-4.93 lv1 a:0x2E3820-0x2E38D8]. The TOC slot
  [OFW-4.93 lv1 a:0x358208] locates the mask word.
- A node holds its next pointer at +48, its split-order key at +56 and
  the four key words at +64 [OFW-4.93 lv1 a:0x2E39B0-0x2E39F8]. It
  holds v1 at +104 and v2 at +112: the walk returns the node plus 64
  [OFW-4.93 lv1 a:0x2E3A00], its caller adds 32
  [OFW-4.93 lv1 a:0x2E3D48], and the caller loads the two values at +8
  and +16 of that [OFW-4.93 lv1 a:0x2E3E2C-0x2E3E40].
- A key with no node answers r3 with the low word 0xFFFFFFFA
  [OFW-4.93 lv1 a:0x2E3E9C-0x2E3EB8].

The walk on the reference console:

| Step | Address | Read |
|---|---|---|
| mask word | 0x373D08 | 0xC0000000 |
| partition table pointer | 0x373D18 | 0x609E00 |
| partition table + 24, the segment list | 0x609E18 | 0x60F140 |
| segment 0 | 0x60F140 | 0x609F60 |
| segment 0 + 72, its entry array | 0x609FA8 | 0x60F270 |
| entry of lpar 1, at index 1 of 8-byte entries | 0x60F278 | 0x66FA30 |
| lpar 1 partition + 56 | 0x66FA68 | 0x687060, the repository |
| bucket 4 of the repository's hash list | 0x687BD8 | 0x688100 |
| node, after three nodes with lower split-order keys | 0x763CF0 | the key words of `ss.param.debug.support` |

The key's hash is 0x31701464 and its split-order key is 0x26280E8D,
which the node holds at +56. The bucket's dummy node holds 0x20000000,
the bit-reversed bucket number.

## What writes the node

lv1 does not create the node itself. lpar 1's update manager creates
it. Its code is in `ss_server1.fself`, one file of the process archive
that lv1 carries [OFW-4.93 lv1 a:0x400000-0x4000E4]:

- `set_debug_support_repository` builds the same key and reads the
  node. When the read succeeds, it modifies the node with its two
  inputs. Otherwise it creates the node with them
  [OFW-4.93 lv1 a:0x4F470C-0x4F4814].
- Its caller reads 16 bytes of parameter data
  [OFW-4.93 lv1 a:0x4F485C]. It sets both values to all-ones before it
  checks anything [OFW-4.93 lv1 a:0x4F4874-0x4F4878].
- It keeps the 16 bytes only when one of three checks of the unit's
  target id passes [OFW-4.93 lv1 a:0x4F4890-0x4F48D4]:
  - the id is 0x82 [OFW-4.93 lv1 a:0x5216FC];
  - the id is 0x81 [OFW-4.93 lv1 a:0x521710];
  - the id is 0x82 with a sub-id of 0x8F or 0x90
    [OFW-4.93 lv1 a:0x521738].
- It then calls `set_debug_support_repository` on every path
  [OFW-4.93 lv1 a:0x4F48E4].

So the node exists on every unit, and both values are all-ones unless
the target id is 0x81 or 0x82. The values on a debug or tool unit come
from that unit's parameter data. This capture does not cover them.

## Files

| File | Rows | Holds |
|---|---|---|
| `node.tsv` | 1 | the firmware, the key, the lv1 address of v1, the number of reads, whether they agree, and v1 and v2 |
| `kernel.tsv` | 2 | the kernel's two doublewords, the number of reads, whether they agree, and the value |

## The values

The node holds v1 = 0xFFFFFFFFFFFFFFFF and v2 = 0xFFFFFFFFFFFFFFFF.
lv1 answers the kernel's read with r3 = 0 and these two values, as the
update manager's code gives for a retail unit. The kernel's two
doublewords hold the same values.

## Cross-checks

- The kernel pages at 0x8000000000050A00 and 0x8000000000349200 are
  the stock 4.93 code and TOC, byte for byte. The read and its TOC
  slots on the console are therefore the shipped ones.
- The walk computed the hash and the bucket from the shipped code, and
  reached the node from that bucket. The node holds the split-order key
  that the same code gives.
