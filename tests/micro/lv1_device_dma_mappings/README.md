# lv1_device_dma_mappings

A read-only capture, in lv1 on the reference console, of every mapping
that lv1 holds in the DMA regions of devices 1 and 2 of bus 1. It gives
the mappings that lv1's `map_device_dma_region` (hypercall 176) made
for the 4.93 kernel's calls for those devices at
[OFW-4.93 lv2_kernel a:0x800000000007D6B4].

CellGov's hypervisor answers those calls through the device bus group
of `cellgov_lv1`. The facts are in `cellgov_ps3_abi::lv1::device_bus`,
whose rows `GAME_OS_DMA_MAPPINGS` hold the kernel's pages. The test
`device_bus::dma_mappings_console_tests` holds the rows, the devices'
words at +160 and the devices' region lists to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the reads
returned, as `cellgov ps3 peek` wrote them.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD. `cellgov ps3 status` passed the profile before the reads.
- On 2026-11-19, after the console had booted, a first walk with
  `cellgov ps3 peek --space lv1` followed the pointers from the system
  bus global at 0x3682C0: the two devices, each device's region list
  through the next link at +32, and each region's mapping list from its
  head at +72 through the next link at +32 of each mapping. Its reads
  are not committed.
- Then five `cellgov ps3 peek --space lv1` commands read the committed
  words at the addresses that the walk gave, one command for each file.
- The command read each page twice, and the two reads of every word
  agree. It wrote nothing to lv1 or lv2, used no poke, sent no hypercall
  and installed nothing on the console.
- Every address is lv1 heap or lv1 data. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No address is MMIO; the reads are lv1's records of the
  regions and mappings, never the devices' registers.
- Only the 211 words below are committed. No console-identifying value
  is in this directory.

## The kernel's calls

When CellGov's hypervisor boots the 4.93 kernel, the network device's
map is followed by 18 calls of 64 KB each, with the flags
0xE000000000000000:

1. device 1: seven pages at the bus addresses 0xC0080000 to 0xC00E0000,
   in its second region;
2. device 2: the same seven pages;
3. device 1: two pages at 0xC0000000 and 0xC0010000, in its first
   region;
4. device 2: the same two pages.

The logical partition addresses are 0x500000320000 to 0x500000370000
for the first six pages of each device's second region, then 0x900000,
0x920000 and 0x930000.

## Where lv1 keeps the mappings

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. `cellgov_ps3_abi::lv1::device_bus`
holds each step with its address.

- The map function's device lookup takes the system bus object's slot 3
  for device 1 and slot 4 for device 2, the doublewords at +56 and +64
  [OFW-4.93 lv1 a:0x235D88-0x235DE8] [OFW-4.93 lv1 a:0x22B838-0x22B844].
  It then calls the device's map function, as for device 0.
- The device's entry at +120 returns -17 when the device's 32-bit word
  at +160 is 16 [OFW-4.93 lv1 a:0x22D718-0x22D738]. Otherwise it gives
  the I/O address as the low 31 bits of the bus address plus the
  device's doublewords at +168 and +176
  [OFW-4.93 lv1 a:0x22D75C-0x22D778].
- Each region keeps its partition at +8, its next link at +32, its bus
  address at +48, its size at +56, its page shift at +64 and the head of
  its mapping list at +72. The README of
  `tests/micro/lv1_device_dma_slots` gives the region layout.
- Each mapping keeps its partition at +8, its links at +24 and +32, the
  offset of its bus address in the region at +40, the logical partition
  address at +48, the length at +56 and the region at +72. The README of
  `tests/micro/lv1_device_dma_map` gives the mapping layout.

## Files

| File | Rows | Holds |
|---|---|---|
| `globals.tsv` | 3 | the system bus global at 0x3682C0, then +56 and +64 of the system bus object |
| `devices.tsv` | 6 | for device 1 and then device 2: the doublewords at +96, +168 and +176 |
| `device_160.tsv` | 2 | the 32-bit word at +160 of device 1 and of device 2 |
| `regions.tsv` | 60 | for each of the ten regions, in list order, device 1 first: +8, +32, +48, +56, +64 and +72 |
| `mappings.tsv` | 140 | for each of the 20 mappings, region by region in list order: +8, +24, +32, +40, +48, +56 and +72 |

Each row holds an address, the number of reads, whether they agree and
the word.

The capture leaves out these words, which the answer does not need:
each region's class at +0 and its words at +16, +24 and +40; each
mapping's class at +0, its copy of the offset at +16, its byte at +64
and its words at +80 and +88; and every other word of the devices. The
network device's one mapping is in `tests/micro/lv1_device_dma_map`.

## The values

- The system bus object is 0x61D710. Its +56 holds device 1, 0x61E050,
  and its +64 holds device 2, 0x61E2A0: the devices of the earlier
  captures `lv1_device_dma_slots` and `lv1_device_2_regions`.
- The word at +160 is 8 for device 1 and 9 for device 2. So the entry
  at +120 gives an I/O address for each device. The doublewords at +168
  are 0x400000000 and 0x500000000, and those at +176 are 0.
- Each device's list holds its five regions of the game OS partition
  (2), with the addresses of the earlier captures, and no region of lv1.
- Each mapping has the partition 2 and links back to its region at +72.
  The lists hold 20 mappings:

  | Device | Region | Mappings | Offsets | Logical partition addresses | Length |
  |---|---|---|---|---|---|
  | 1 and 2 | 0xC0000000 | 2 | 0x10000, 0 | 0x930000, 0x920000 | 0x10000 |
  | 1 and 2 | 0xC0080000 | 7 | 0x60000 down to 0 | 0x900000, then 0x500000370000 down to 0x500000320000 | 0x10000 |
  | 1 | 0xC0800000 | 1 | 0 | 0x9A0000 | 0x10000 |
  | 2 | 0xD0000000 | 1 | 0 | 0x800000 | 0x100000 |

  The other regions' lists are empty.
- So each of the kernel's 18 calls has its mapping in the list of its
  device's region. In each list the head is the mapping of the region's
  last call, and each next link goes to the mapping of the call before.
- The two last rows of the table are mappings of calls that CellGov's
  boot of the kernel has not yet reached.
