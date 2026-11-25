# lv1_device_dma_segments

A read-only capture, in lv1 on the reference console, of the address
allocators that lv1 uses for the DMA regions of devices 0, 1 and 2 of
bus 1, their blocks, and lv1's global I/O segment table. The capture
shows that each device takes its bus addresses from I/O segments of its
own, and that the segments of two devices do not overlap.

`cellgov_ps3_abi::lv1::device_bus` gives lv1's path with each address.
The test `dma_segments_console_tests` holds the path's facts to this
capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words that the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-23, `cellgov ps3 peek --space lv1` read the words below
  through webMAN's `peek.lv1` command, after the console had booted to
  the XMB. Each read took the address that the read before it gave. It
  read each page twice, and the two reads of every word agree. It wrote
  nothing to lv1 or lv2, used no poke, sent no hypercall and installed
  nothing on the console.
- Every address is lv1 heap or lv1 data. No address is MMIO. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No page that a read touched covers the body of the
  region access check's ACL comparison
  [OFW-4.93 lv1 a:0x25B6F0-0x25B7C8].
- The capture read the walk one step at a time. Then it read the same
  addresses again in the commands below. The two reads agree. Only the
  second reads are committed.
- The capture leaves out two kinds of word. It does not read +0 of the
  segment table's entry, which holds a register address. It does not
  commit the doubleword at +128 of each pool, whose low half is a word
  that no read explains.
- A field that lv1 reads as a 32-bit word was read with `--width 4`.
- No console-identifying value is in this directory.

## lv1's path

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. S is the system bus object, and
`cellgov_ps3_abi::lv1::device_bus` gives the citations.

- The pool of a device is the doubleword at S + 32 + 8 x index. The
  word k at +160 of the pool selects the device's allocator.
- For the partition's calls, the allocator is the doubleword at
  S + 328 + 8 x k. lv1's own calls take S + 192 + 8 x k.
- An allocator holds its method table at +0, its first range at +8 and
  +16, its block list at +24, its second range at +32 and +40, and a
  word at +48.
- A block holds the next block at +32, its start at +40, its size at
  +48, and its page shift at +56.
- Each block holds the word 2 at +8. The new-range method takes the
  first range for a word other than 1. The new range is the first free
  I/O segment of that range.
- The address step subtracts the pool's words at +168 and +176 from the
  found address and adds 0x80000000.
- The segment table's entry count is the word at the address in the TOC
  slot r2 - 8480 (0x357F18). The table is the doubleword at the address
  in the TOC slot r2 - 8488 (0x357F10). Entry 0's bitmap object is at
  +64 of the entry. A set bit marks a free segment.

## The values

| Read | Address | Value | What it gives |
|---|---|---|---|
| TOC slot r2 - 8480 | 0x357F18 | 0x373D70 | the entry count's word |
| TOC slot r2 - 8488 | 0x357F10 | 0x373D78 | the table's global |
| the entry count | 0x373D70 | word 1 | one entry |
| the table | 0x373D78 | 0x609FE0 | the table |
| entry 0 | 0x609FE0 | 0x60FE10 | P |
| P + 16 | 0x60FE20 | word 35 | the entry's size shift |
| bitmap + 32, + 36 | 0x60FE70 | words 6 and 37 | chunk 0 holds segments 0 to 63, chunk 1 segments 64 to 127 |
| S | 0x3682C0 | 0x61D710 | the system bus object |
| S + 32, + 56, + 64 | 0x61D730 | 0x61D900, 0x61E050, 0x61E2A0 | the pools of devices 0, 1 and 2 |
| pool + 160 | | words 2, 8 and 9 | k of each device |
| pool + 136 | | S for each device | the backend |
| pool + 168, + 176 | | 0x1_0000_0000, 0x4_0000_0000, 0x5_0000_0000; 0 | the address step's words |
| S + 192 + 8 x k | 0x61D7D0 | allocators for k = 2, 10 and 11 only | lv1's own allocators |
| S + 328 + 8 x k | 0x61D858 | allocators for k = 2, 8, 9, 10 and 11 | the partition's allocators |
| allocator of k = 2 | 0x61EAC0 | class 0x35C760, 0x1_4000_0000 / 0x4000_0000, 0x1_2000_0000 / 0x2000_0000, word 0 | device 0 |
| allocator of k = 8 | 0x61EB80 | class 0x35C760, 0x4_4000_0000 / 0x4000_0000, 0x4_2000_0000 / 0x2000_0000, word 0x80 | device 1 |
| allocator of k = 9 | 0x61EBC0 | class 0x35C760, 0x5_4000_0000 / 0x4000_0000, 0x5_2000_0000 / 0x2000_0000, word 0x80 | device 2 |
| bitmap word 0 of chunk 0 | 0x61D4E0 | 0xFFE3FFFBFFEBC0FE | segments 0x00, 0x08 to 0x0D, 0x12, 0x14, 0x22 and 0x32 to 0x34 held |
| bitmap word 0 of chunk 1 | 0x61F790 | 0xFFECFFFFFF8FFF8F | segments 0x44 to 0x46, 0x54 to 0x56, 0x70, 0x71 and 0x74 held |

The blocks:

| Device | Block | Start | Size | Page shift |
|---|---|---|---|---|
| 0 | 0x38B2D0 | 0x1_4000_0000 | 0x10_0000 | 20 |
| 1 | 0x1DFBE0 | 0x4_4000_0000 | 0x200_0000 | 16 |
| 1 | 0x530EA0 | 0x4_5000_0000 | 0x1000_0000 | 20 |
| 1 | 0x531790 | 0x4_6000_0000 | 0x80_0000 | 12 |
| 2 | 0x5336C0 | 0x5_4000_0000 | 0x200_0000 | 16 |
| 2 | 0x53C970 | 0x5_5000_0000 | 0x1000_0000 | 20 |
| 2 | 0x53D260 | 0x5_6000_0000 | 0x80_0000 | 12 |

- Each device's blocks lie in its first range, and each block's segment
  is held in the bitmap: 0x14 for device 0, 0x44 to 0x46 for device 1
  and 0x54 to 0x56 for device 2.
- The segments of device 0's ranges are 0x12 to 0x17, of device 1's
  0x42 to 0x47 and of device 2's 0x52 to 0x57. They do not overlap.
- Each region of `GAME_OS_DMA_REGIONS` maps into a block of its own
  device. For device 1, 0xC0000000 - 0x80000000 + 0x4_0000_0000 is
  0x4_4000_0000, the start of its first block.
- Segment 0x12 holds lv1's own region of device 0, at bus address
  0xA0000000. That region is in lv1's own allocator of k = 2, so a close
  of device 0 does not free it.

## Files

| File | Rows | Holds |
|---|---|---|
| `globals.tsv` | 3 | the two TOC slots and the system bus global |
| `count.tsv` | 1 | the entry count |
| `table.tsv` | 2 | the table's global and entry 0 |
| `segment_words.tsv` | 3 | P + 16 and the bitmap object's words at +32 and +36 |
| `chunks.tsv` | 5 | the chunk array pointer, chunks 0 and 1, and each chunk's bit words pointer |
| `bitmap.tsv` | 2 | bit word 0 of chunks 0 and 1 |
| `pools.tsv` | 3 | the pools of devices 0, 1 and 2 |
| `pool_kind.tsv` | 3 | +160 of each pool |
| `pool_fields.tsv` | 9 | +136, +168 and +176 of each pool |
| `lv1_allocators.tsv` | 15 | S + 192 + 8 x k for k = 0 to 14 |
| `allocators.tsv` | 15 | S + 328 + 8 x k for k = 0 to 14 |
| `allocator.tsv` | 21 | +0 to +48 of the allocators of k = 2, 8 and 9 |
| `blocks.tsv` | 84 | +0 to +88 of each of the seven blocks |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x357F18,0x357F10,0x3682C0 --out globals.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x373D70 --out count.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x373D78,0x609FE0 --out table.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61D730,0x61D748,0x61D750 --out pools.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x61D9A0,0x61E0F0,0x61E340 --out pool_kind.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61D988,0x61D9A8,0x61D9B0,0x61E0D8,0x61E0F8,0x61E100,0x61E328,0x61E348,0x61E350 --out pool_fields.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61D7D0,0x61D7D8,...,0x61D840 --out lv1_allocators.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61D858,0x61D860,...,0x61D8C8 --out allocators.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61EAC0,...,0x61EAF0,0x61EB80,...,0x61EBB0,0x61EBC0,...,0x61EBF0 --out allocator.tsv
cellgov ps3 peek --space lv1 --width 8 --address <each block + 0 to + 88> --out blocks.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x60FE20,0x60FE70,0x60FE74 --out segment_words.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x60FE68,0x610340,0x610348,0x61D4C0,0x61F770 --out chunks.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61D4E0,0x61F790 --out bitmap.tsv
```

The elided lists read each doubleword in steps of 8. `blocks.tsv` reads
the seven blocks in the order of the table above.
