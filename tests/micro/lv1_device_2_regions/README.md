# lv1_device_2_regions

A read-only capture, in lv1 on the reference console, of the MMIO and
DMA regions that lv1 holds for device 2 of bus 1, and of the mappings
that lv1 holds in its MMIO regions. It gives the mappings that lv1's
`map_device_mmio_region` (hypercall 172) made for the 4.93 kernel's
calls for device 2 at [OFW-4.93 lv2_kernel a:0x800000000007D8CC].

CellGov's hypervisor answers those calls through the device bus group
of `cellgov_lv1`. The facts are in `cellgov_ps3_abi::lv1::device_bus`,
and the test `device_bus_mmio_device_2_console_tests` holds them to this
capture. The DMA regions and slots are read for the kernel's later
`allocate_device_dma_region` calls for the device; no test reads them
yet.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the reads
returned, as `cellgov ps3 peek` wrote them.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48. `cellgov ps3 status` passed the profile before
  the reads.
- On 2026-11-19, `cellgov ps3 peek --space lv1` read the words below
  through webMAN's `peek.lv1` command, after the console had booted.
  The reads ran in seven steps, each from the values that the steps
  before it returned:
  1. the system bus global at 0x3682C0, and entries 0 and 1 of the
     table of slot objects at 0x3682F8;
  2. +64 of the system bus object, and +24 of device 2's slot object;
  3. the device's doublewords, its open byte, and the mark, address
     word and partition of each of the slot object's six slots;
  4. +0 to +72 of each MMIO region that a device slot holds, and of the
     head of the device's DMA region list;
  5. the 12 doublewords of the mapping at the head of each MMIO
     region's list, and the next DMA region;
  6. the 12 doublewords of the mapping that each head's next link
     gives, the next DMA region, and the mappings at the heads of the
     lists of the first two DMA regions;
  7. the last two DMA regions.
- The command read each page twice, and the two reads of every word
  agree. It wrote nothing to lv1 or lv2, used no poke, sent no hypercall
  and installed nothing on the console.
- Every address is lv1 heap or lv1 data. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No address is MMIO; the reads are lv1's records of the
  regions, never the regions.
- Only the 202 words below are committed. The DMA mapping lists hold
  further mappings, which the reads do not follow.
- No console-identifying value is in this directory.

## The kernel's calls

When CellGov's hypervisor boots the 4.93 kernel, the first call for
device 2 has r3 = 1, r4 = 2, r5 = 0x3020000, r6 = 0x10000 and
r7 = 12: a map of 64 KB of 4 KB pages at the bus address 0x3020000.
It comes after the kernel's two calls for device 1, whose records
`tests/micro/lv1_device_mmio_map` and `tests/micro/lv1_device_mmio_lists`
read.

## Where lv1 keeps the regions

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. `cellgov_ps3_abi::lv1::device_bus`
holds each step with its address.

- lv1's init makes device 2 with the constructor of device 1 and the
  index 4, and stores it at +64 of the system bus object
  [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AC4]. The constructor stores the index
  at +128 of the device [OFW-4.93 lv1 a:0x3A9768-0x3A9780]
  [OFW-4.93 lv1 a:0x22DD94] [OFW-4.93 lv1 a:0x22DDD8].
- The init then gives the device three MMIO regions
  [OFW-4.93 lv1 a:0x3A9AB4-0x3A9AF0], in the slots at +24 + 8 x i.
  The region and mapping objects have the layouts that the README of
  `tests/micro/lv1_device_mmio_map` gives.
- The real address of an MMIO region is 0x24000000000 plus its bus
  address. lv1's address function also adds a word of the bus object
  only for the index 6 [OFW-4.93 lv1 a:0x22BE1C-0x22BE68].
- Device 2's slot object is entry 1 of the table at 0x3682F8
  [OFW-4.93 lv1 a:0x23710C-0x237120]. Its slots have the layout that the
  README of `tests/micro/lv1_device_dma_slots` gives.

## Files

| File | Rows | Holds |
|---|---|---|
| `globals.tsv` | 3 | the system bus global at 0x3682C0, then entries 0 and 1 of the table of slot objects |
| `device_pointers.tsv` | 2 | +64 of the system bus object, then +24 of device 2's slot object |
| `device.tsv` | 14 | the device's doublewords at +0, +24 to +80 (its 8 MMIO region slots), +96, +112, +120, +128 and +136 |
| `device_open.tsv` | 1 | the device's open byte at +108 |
| `slot_valid.tsv` | 6 | the mark byte of slots 0 to 5 |
| `slot_address.tsv` | 6 | the address word of slots 0 to 5 |
| `slot_partition.tsv` | 6 | the partition of slots 0 to 5 |
| `mmio_regions.tsv` | 30 | +0 to +72 of the regions in device slots 0, 1 and 2 |
| `mmio_heads.tsv` | 36 | the 12 doublewords of the mapping at the head of the list of each of the three regions, in slot order |
| `mmio_next.tsv` | 24 | the 12 doublewords of the mapping that the next link of the second and third regions' heads gives |
| `dma_region_1.tsv` to `dma_region_5.tsv` | 10 each | +0 to +72 of each region of the device's DMA list, in list order |
| `dma_mapping_heads.tsv` | 24 | the 12 doublewords of the mapping at the head of the list of DMA regions 1 and 2 |

Each row holds an address, the number of reads, whether they agree and
the word.

## The values

- Both +64 of the system bus object and +24 of entry 1 of the table
  hold the device 0x61E2A0. So the access check, the map path and the
  slot path use the same device.
- The device's method table is 0x35C878, the table of devices 0 and 1.
  Its open byte is 1 and +112 holds 2, so the game OS partition holds
  the device open. The word at +128 is 4, the index of lv1's init, and
  +136 holds the system bus object.
- Slots 0 to 2 hold three regions, and slots 3 to 7 are empty. The
  regions have the class 0x35C560 and the partition 0. Their bus
  addresses and sizes are those of lv1's init: 0x2600 and 0x200,
  0x3020000 and 0x10000, and 0x3820000 and 0x10000. Each page shift is
  20.
- The second region's list holds two mappings:
  - The head has the game OS partition 2, the logical partition address
    0x400000090000 at +48 and +80, and the length 0x10000. Its byte at
    +64 is 1.
  - The next has the partition 0 and the address 0x24003020000, and
    its next link is 0.
- The third region's list has the same shape, with the game OS address
  0x4000000A0000 and lv1's address 0x24003820000.
- The first region's list holds only lv1's mapping, with the address
  0x24000002600. So the game OS does not map the first region.
- The game OS addresses are those of the region counter values 9 and
  10, the two values after device 1's mappings (7 and 8).
- The DMA list holds five regions of the game OS partition, with the
  addresses, sizes and page shifts of device 1's five regions, in the
  same order. Slots 0 to 4 are taken, each with the partition 2 and the
  low word of the address of the region at the same place in the list,
  and slot 5 is free.
