# lv1_device_dma_region

A read-only capture, in lv1 on the reference console, of the DMA
regions that lv1 holds for device 0 of bus 1, the network device. It
gives the bus address that lv1's `allocate_device_dma_region`
(hypercall 174) gave the 4.93 kernel's call at
[OFW-4.93 lv2_kernel a:0x800000000009585c].

CellGov's hypervisor answers that call with this address through
`GAME_OS_DMA_REGIONS` in `cellgov_ps3_abi::lv1::device_bus`. The test
`device_bus_dma_region_console_tests` holds the row to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-17, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. Each read took the address that the
  read before it gave. It read each page twice, and the two reads of
  every word agree. It wrote nothing to lv1 or lv2, used no poke, sent
  no hypercall and installed nothing on the console.
- Every address is lv1 heap or lv1 data. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No address is MMIO.
- Only the 30 words below are committed.
- No console-identifying value is in this directory.

## The kernel's call

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps:

- The function at 0x80000000000aa3fc gives the wrapper the bus id at
  +144 of its device record in r3, the device id at +152 in r4, a size
  in r5, a page size in r6, 0 in r7 and an output address in r8
  [OFW-4.93 lv2_kernel a:0x80000000000aa488-0x80000000000aa4a0].
- The wrapper turns the page size into its base-2 logarithm: 4 KB, 64 KB,
  1 MB and 16 MB give 12, 16, 20 and 24
  [OFW-4.93 lv2_kernel a:0x8000000000095794-0x80000000000957fc]
  [OFW-4.93 lv2_kernel a:0x8000000000095840-0x8000000000095844].
- It sends hypercall 174 with r3 to r7 in that order
  [OFW-4.93 lv2_kernel a:0x8000000000095848-0x800000000009585c].
- When r3 is 0, it stores r4 at the output address and returns 0.
  Otherwise it returns 0x8001000C
  [OFW-4.93 lv2_kernel a:0x8000000000095860-0x8000000000095888].

When CellGov's hypervisor boots that kernel, the call has r3 = 1,
r4 = 0, r5 = 0x100000, r6 = 20 and r7 = 0: a 1 MB region of 1 MB pages
for the network device.

## Where lv1 keeps the regions

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. `cellgov_ps3_abi::lv1::device_bus`
holds each step with its address.

- The getter of the system bus object reads the doubleword that the TOC
  slot at r2 - 27872 locates. The slot holds 0x3682C0
  [OFW-4.93 lv1 a:0x22B6BC-0x22B6C4] [OFW-4.93 lv1 a:0x353358-0x35335C].
  Device 0 of bus 1 is the doubleword at +32 of that object
  [OFW-4.93 lv1 a:0x22B838-0x22B844].
- The allocator makes an 80-byte region object
  [OFW-4.93 lv1 a:0x226C5C-0x226C70] and fills it
  [OFW-4.93 lv1 a:0x225CAC-0x225CDC]:
  - +0 holds the class of the TOC slot r2 - 28312, which holds
    0x35C560 [OFW-4.93 lv1 a:0x3531A0-0x3531A4];
  - +8 holds the calling partition;
  - +16 and +48 hold the bus address;
  - +56 holds the size and +64 the page shift of the call;
  - +24 and +32 link the region to the one before and the one after it.
- The allocator links the region into the device's list, whose head is
  at +96 of the device [OFW-4.93 lv1 a:0x226C90-0x226D04].
- The handler gives the caller +48 of the region in r4
  [OFW-4.93 lv1 a:0x2268EC-0x2268F4] [OFW-4.93 lv1 a:0x23707C-0x237088].

## Files

| File | Rows | Holds |
|---|---|---|
| `dma_region.tsv` | 30 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields:

- `system_bus` and `device`: the global at 0x3682C0 and +32 of the
  system bus object.
- `device_table`, `device_regions`, `device_open`, `device_partition`,
  `device_120`, `device_index`, `device_backend` and `device_kind`: the
  device's doublewords at +0, +96, +104, +112, +120, +128, +136 and +160.
- `region_N_class`, `region_N_partition`, `region_N_base_16`,
  `region_N_previous`, `region_N_next`, `region_N_40`,
  `region_N_address`, `region_N_size`, `region_N_page_shift` and
  `region_N_72`: the doublewords at +0 to +72 of the Nth region of the
  list.

## The live regions

- The device's method table is 0x35C878, the table of device 0. The
  byte at +108 is 1 and +112 holds 2, so the game OS partition holds the
  device open (see `tests/micro/lv1_device_bus_open`).
- +120 of the device is 2. The allocator refuses a device whose +120
  is -1 [OFW-4.93 lv1 a:0x22D990-0x22D9B0], so it accepts this device. The word at +128 is 0 and +136 holds the system bus object, so
  the device takes its addresses from slot 0 of that object, which is
  the device itself. The word at +160 is 2.
- The list holds two regions, and both have the class 0x35C560.
- Region 1 has the partition 0, the bus address 0xA0000000, the size
  0x8000 and the page shift 12. It is not a region of the game OS
  partition.
- Region 2 has the partition 2, the bus address 0xC0000000, the size
  0x100000 and the page shift 20. Its previous link is region 1 and its
  next link is 0, so the list ends there.
- Region 2 is the only region of the game OS partition, and its size
  and page shift are those of the kernel's call. So lv1 gave that call
  the bus address 0xC0000000.
- The word at +72 of each region is not 0. The allocator stores 0 there
  [OFW-4.93 lv1 a:0x225CB8], so a later call set it.
