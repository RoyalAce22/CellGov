# lv1_device_dma_slots

A read-only capture, in lv1 on the reference console, of the DMA
regions that lv1 holds for device 1 of bus 1. It gives the bus address
that lv1's `allocate_device_dma_region` (hypercall 174) gave the 4.93
kernel's call for that device at
[OFW-4.93 lv2_kernel a:0x800000000009585C].

CellGov's hypervisor answers that call and the kernel's four later
calls for the device with the five addresses of the device's list, in
list order, through `GAME_OS_DMA_REGIONS` in
`cellgov_ps3_abi::lv1::device_bus`. The tests
`device_bus_dma_slots_console_tests` and
`device_bus_dma_slot_rows_console_tests` hold the rows to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the reads
returned, as `cellgov ps3 peek` wrote them.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD. `cellgov ps3 status` passed the profile before the reads.
- On 2026-11-19, `cellgov ps3 peek --space lv1` read the words below
  through webMAN's `peek.lv1` command, after the console had booted to
  the XMB. Earlier reads in the same session followed the pointers; the
  committed reads take the addresses that those reads gave.
- The command read each page twice, and the two reads of every word
  agree. It wrote nothing to lv1 or lv2, used no poke, sent no hypercall
  and installed nothing on the console.
- Every address is lv1 heap or lv1 data. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No address is MMIO; the reads are lv1's records, never
  the device's registers.
- Only the 77 words below are committed. The reads of each slot skip
  the three bytes between its mark and its address.
- No console-identifying value is in this directory.

## The kernel's call

When CellGov's hypervisor boots the 4.93 kernel, the call has r3 = 1,
r4 = 1, r5 = 0x40000, r6 = 16 and r7 = 1: a 256 KB region of 64 KB
pages for device 1, with flags 1. The kernel's wrapper sends r3 to r7
in that order [OFW-4.93 lv2_kernel a:0x8000000000095848-0x800000000009585C].

## Where lv1 keeps the regions

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. `cellgov_ps3_abi::lv1::device_bus`
holds each step with its address.

- lv1's allocate function sends devices 1 and 2 of bus 1 to a path of
  their own [OFW-4.93 lv1 a:0x237108-0x23711C]. The path takes entry 0
  (device 1) or entry 1 (device 2) of a table of slot objects
  [OFW-4.93 lv1 a:0x23D0F8-0x23D104]. The TOC slot at r2 - 25520 holds
  the table's address, 0x3682F8 [OFW-4.93 lv1 a:0x353C88-0x353C8C].
- The slot function calls the device allocate function for the device
  at +24 of the slot object [OFW-4.93 lv1 a:0x23D524-0x23D540]. That
  function makes an 80-byte region object and links it into the
  device's list at +96, as for device 0 (see
  `tests/micro/lv1_device_dma_region`).
- The slot function then takes the first free slot of six. Slot i holds
  a mark byte at +104 + 16i, the low word of the address at +108 + 16i
  and the partition at +112 + 16i [OFW-4.93 lv1 a:0x23D54C-0x23D5FC].
- The access check finds device 1 through code 3
  [OFW-4.93 lv1 a:0x235DA4-0x235DAC], the doubleword at +56 of the
  system bus object [OFW-4.93 lv1 a:0x22B838-0x22B844].

## Files

| File | Rows | Holds |
|---|---|---|
| `device_table.tsv` | 1 | entry 0 of the table of slot objects, at 0x3682F8 |
| `wrapper.tsv` | 6 | +24 of device 1's slot object, then the partition of slots 0 to 4 |
| `slot_valid.tsv` | 6 | the mark byte of slots 0 to 5 |
| `slot_address.tsv` | 5 | the low word of the address of slots 0 to 4 |
| `device.tsv` | 6 | the device's doublewords at +0, +96, +112, +120, +128 and +136 |
| `device_open.tsv` | 1 | the device's open byte at +108 |
| `system_bus.tsv` | 2 | the global at 0x3682C0 and +56 of the system bus object |
| `regions.tsv` | 50 | the doublewords at +0 to +72 of each of the five regions of the device's list, in list order |

Each row holds an address, the number of reads, whether they agree and
the word.

## The values

- Entry 0 of the table is the slot object 0x7D920. Its +24 is the
  device 0x61E050, which is also +56 of the system bus object 0x61D710.
  So the slot path and the access check use the same device.
- The device's method table is 0x35C878, the table of device 0. Its
  open byte is 1 and +112 holds 2, so the game OS partition holds the
  device open. +120 is 5, so the allocator accepts the device. The word
  at +128 is 3, and +136 holds the system bus object, so the device
  takes its addresses from slot 3 of that object, the device itself.
- The list holds five regions, all of the game OS partition (2), with
  the class 0x35C560:

  | Region | Address | Size | Page shift |
  |---|---|---|---|
  | 1 | 0xC0000000 | 0x40000 | 16 |
  | 2 | 0xC0080000 | 0x80000 | 16 |
  | 3 | 0xC0800000 | 0x800000 | 16 |
  | 4 | 0xD0000000 | 0x800000 | 20 |
  | 5 | 0xE0000000 | 0x10000 | 12 |

- Region 1 is the head of the list: its previous link is 0. Each
  region's previous link is the region before it, and region 5's next
  link is 0.
- Slots 0 to 4 are taken, each with the partition 2 and the low word of
  the address of the region at the same place in the list. Slot 5 is
  free. The slot function takes the first free slot, so the slots and
  the list are in the order of the calls.
- Region 1 is the only region with the size and page shift of the
  kernel's call, and it is the first. So lv1 gave that call the bus
  address 0xC0000000.
