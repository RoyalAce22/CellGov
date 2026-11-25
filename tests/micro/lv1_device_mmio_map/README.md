# lv1_device_mmio_map

A read-only capture, in lv1 on the reference console, of the MMIO
regions of device 1 of bus 1 and of the mappings that lv1 holds in the
second region. It gives the mapping that lv1's `map_device_mmio_region`
(hypercall 172) made for the 4.93 kernel's call at
[OFW-4.93 lv2_kernel a:0x800000000007D8CC].

CellGov's hypervisor answers that call through the device bus group of
`cellgov_lv1`. The facts are in `cellgov_ps3_abi::lv1::device_bus`, and
the test `device_bus_mmio_map_console_tests` holds them to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48. `cellgov ps3 status` passed the profile before
  the reads.
- On 2026-11-19, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. The reads ran in four commands, each
  from the values that the one before it returned:
  1. the device's method table and its 8 region slots, at the device
     address that `tests/micro/lv1_device_bus_open` gives for slot 3;
  2. +0 to +72 of each region that a slot holds;
  3. the 12 doublewords of the mapping at +72 of region 2;
  4. the 12 doublewords of the mapping that its next link gives.
- The command read each page twice, and the two reads of every word
  agree. It wrote nothing to lv1 or lv2, used no poke, sent no hypercall
  and installed nothing on the console.
- Every address is lv1 heap. No row of `tests/micro/console_crashes.tsv`
  covers these addresses: each row is an lv2 range. No address is MMIO;
  the reads are lv1's records of the regions, never the regions.
- Only the 63 words below are committed.
- No console-identifying value is in this directory.

## The kernel's call

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the wrapper's steps:

- It keeps the bus id, the device id and the size from r3, r4 and r6
  [OFW-4.93 lv2_kernel a:0x800000000007D860-0x800000000007D868].
- For a flags word in r7 that sets none of the tested bits, it sends
  hypercall 172 with r7 = 12 and its own r3, r4, r5 and r6
  [OFW-4.93 lv2_kernel a:0x800000000007D894-0x800000000007D8CC].
- When r3 is 0, it keeps r4 as the logical partition address
  [OFW-4.93 lv2_kernel a:0x800000000007D8D0-0x800000000007D8DC], and
  maps that address into its own address space
  [OFW-4.93 lv2_kernel a:0x800000000007D958-0x800000000007D978].
- When that map fails, it unmaps the region with hypercall 173
  [OFW-4.93 lv2_kernel a:0x800000000007D988-0x800000000007D998].

CellGov's run of the 4.93 kernel reached the call with bus 1, device 1,
the bus address 0x3010000, the size 0x10000 and the page shift 12.

## Where lv1 keeps the regions and the mappings

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. `cellgov_ps3_abi::lv1::device_bus`
holds each step with its address.

- lv1's init gives device 1 three MMIO regions
  [OFW-4.93 lv1 a:0x3A9A60-0x3A9A9C]. The region function stores region
  i in the slot at +24 + 8 x i of the device. Each region is an 80-byte
  object with the page shift 20 [OFW-4.93 lv1 a:0x227F4C-0x227FC8].
- The region object has the layout of a DMA region
  [OFW-4.93 lv1 a:0x225CAC-0x225CDC]: the class at +0, the partition at
  +8, the bus address at +16 and +48, the size at +56, the page shift at
  +64 and the head of its mapping list at +72.
- The map function finds the region whose bus address at +48 is the
  call's [OFW-4.93 lv1 a:0x228C70-0x228CA0], and makes the mapping with
  the mapping function of the DMA path [OFW-4.93 lv1 a:0x228D90-0x228DB8].
  That function fills a 96-byte mapping object
  [OFW-4.93 lv1 a:0x225E60-0x225EB4] and links it into the region's
  list [OFW-4.93 lv1 a:0x225EB8-0x225F50]. The map function then stores
  the logical partition address at +80 of the mapping
  [OFW-4.93 lv1 a:0x228DC8-0x228DD4].

## Files

| File | Rows | Holds |
|---|---|---|
| `mmio_map.tsv` | 63 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields:

- `device_table`: the doubleword at +0 of device 1.
- `device_slot_0` to `device_slot_7`: the doublewords at +24 to +80 of
  device 1, its MMIO region slots.
- `region_N_class`, `region_N_partition`, `region_N_base_16`,
  `region_N_previous`, `region_N_next`, `region_N_40`,
  `region_N_address`, `region_N_size`, `region_N_page_shift` and
  `region_N_72`: the doublewords at +0 to +72 of the region in slot
  N - 1.
- `mapping_N_class`, `mapping_N_partition`, `mapping_N_offset_16`,
  `mapping_N_previous`, `mapping_N_next`, `mapping_N_offset`,
  `mapping_N_lpar`, `mapping_N_length`, `mapping_N_64`,
  `mapping_N_region`, `mapping_N_lpar_80` and `mapping_N_88`: the
  doublewords at +0 to +88 of the Nth mapping of region 2's list.

## The values

- The device has the constructor's method table, as in
  `tests/micro/lv1_device_bus_open`.
- Slots 0 to 2 hold three regions, and slots 3 to 7 are empty. The
  regions have the class 0x35C560 and the partition 0. Their bus
  addresses and sizes are those of lv1's init: 0x2400 and 0x200,
  0x3010000 and 0x10000, and 0x3810000 and 0x10000. Each page shift is
  20.
- Region 2 holds the call's bus address. Its list holds two mappings:
  - Mapping 1, at the head, has the partition 2 of the game OS, the
    offset 0, the logical partition address 0x400000070000 at +48 and
    +80, and the length 0x10000. Its byte at +64 is 1.
  - Mapping 2 has the partition 0, lv1's own, and the logical partition
    address 0x24003010000. Its byte at +64 is 0.
- So the game OS holds one mapping of region 2. The map function writes
  +80 only on the path that answers 0. So lv1 answered the kernel's call
  with 0 and the logical partition address 0x400000070000.
