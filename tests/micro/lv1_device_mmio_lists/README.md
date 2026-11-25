# lv1_device_mmio_lists

A read-only capture, in lv1 on the reference console, of the mappings
that lv1 holds in the first and third MMIO regions of device 1 of bus
1. It gives the mapping that lv1's `map_device_mmio_region` (hypercall
172) made for the 4.93 kernel's second call at
[OFW-4.93 lv2_kernel a:0x800000000007D8CC], of the region at the bus
address 0x3810000.

`tests/micro/lv1_device_mmio_map` reads the device's three regions and
the mappings of the second region. This capture follows the mapping
lists that it did not read.

CellGov's hypervisor answers the call through the device bus group of
`cellgov_lv1`. The facts are in `cellgov_ps3_abi::lv1::device_bus`, and
the test `device_bus_mmio_lists_console_tests` holds them to this
capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the reads
returned, as `cellgov ps3 peek` wrote them.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD. `cellgov ps3 status` passed the profile before the reads.
- On 2026-11-19, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. The reads ran in two commands, the
  second from the values that the first returned:
  1. the list head at +72 of each of the three regions, and the 12
     doublewords at the list heads of the first and third regions;
  2. the 12 doublewords of the mapping that the next link of the third
     region's head gives.
- The command read each page twice, and the two reads of every word
  agree. It wrote nothing to lv1 or lv2, used no poke, sent no hypercall
  and installed nothing on the console.
- Every address is lv1 heap. No row of `tests/micro/console_crashes.tsv`
  covers these addresses: each row is an lv2 range. No address is MMIO;
  the reads are lv1's records of the mappings, never the regions.
- Only the 39 words below are committed.
- No console-identifying value is in this directory.

## Files

| File | Rows | Holds |
|---|---|---|
| `region_lists.tsv` | 27 | the list heads of the three regions, then the mapping at the head of the third region's list, then the mapping at the head of the first region's list |
| `region_3_next.tsv` | 12 | the mapping that the next link of the third region's head gives |

Each row holds an address, the number of reads, whether they agree and
the doubleword. The mapping object has the layout that the README of
`tests/micro/lv1_device_mmio_map` gives: the class at +0, the partition
at +8, the previous and next links at +24 and +32, the logical partition
address at +48 and +80, the length at +56, the byte at +64 and the
region at +72.

## The values

- The list heads at +72 of the three regions are those that
  `tests/micro/lv1_device_mmio_map` read: 0x7DA20, 0x387F00 and
  0x1DE260.
- The third region's list holds two mappings:
  - The head, at 0x1DE260, has the partition 2 of the game OS, the
    logical partition address 0x400000080000 at +48 and +80, the length
    0x10000 and the third region at +72. Its byte at +64 is 1.
  - The next mapping, at 0x7DB00, has the partition 0, lv1's own, and
    the logical partition address 0x24003810000. Its byte at +64 is 0,
    and its next link is 0.
- The first region's list holds one mapping, at 0x7DA20, with the
  partition 0 and the logical partition address 0x24000002400. Its next
  link is 0.
- So the game OS holds one mapping of the third region and none of the
  first. The map function writes +80 only on the path that answers 0. So
  lv1 answered the kernel's second call with 0 and the logical partition
  address 0x400000080000.
- In the encoding of `cellgov_ps3_abi::lv1::memory`, 0x400000070000 and
  0x400000080000 are the regions of 64 KB with the counter values 7 and
  8. In CellGov's run of the 4.93 kernel, the regions before the first
  call take the values 2 to 6, and the two calls come next.
