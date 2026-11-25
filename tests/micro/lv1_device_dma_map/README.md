# lv1_device_dma_map

A read-only capture, in lv1 on the reference console, of the mapping
that lv1 holds in the network device's game OS DMA region. It gives the
mapping that lv1's `map_device_dma_region` (hypercall 176) made for the
4.93 kernel's call at [OFW-4.93 lv2_kernel a:0x800000000007d6b4].

CellGov's hypervisor answers that call through the device bus group of
`cellgov_lv1`. The facts are in `cellgov_ps3_abi::lv1::device_bus`, and
the test `device_bus_dma_map_console_tests` holds them to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-17, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. The first read is the word at +72 of
  region 2 of `tests/micro/lv1_device_dma_region`, and the other reads
  are the 12 doublewords at the address that it gave. It read each page
  twice, and the two reads of every word agree. It wrote nothing to lv1
  or lv2, used no poke, sent no hypercall and installed nothing on the
  console.
- Every address is lv1 heap. No row of `tests/micro/console_crashes.tsv`
  covers these addresses: each row is an lv2 range. No address is MMIO.
- Only the 13 words below are committed.
- No console-identifying value is in this directory.

## The kernel's call

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps:

- The function at 0x800000000009541c gives the wrapper the bus id at
  +80 and the device id at +88 of its device record, the bus address, the
  buffer's address, the length, a page size and 0
  [OFW-4.93 lv2_kernel a:0x80000000000954a4-0x80000000000954d4].
- With 0 in r9, the wrapper sets the IOPTE flags to 0xE000000000000000
  [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc].
- For each page of the buffer, the wrapper turns the page's address into
  a logical partition address and sends hypercall 176 with the bus id,
  the device id, that address, the bus address of the page, the page size
  and the flags [OFW-4.93 lv2_kernel a:0x800000000007d678-0x800000000007d6b4].
- A status of -9 or -2 makes it unmap the pages that it mapped, with
  hypercall 177 [OFW-4.93 lv2_kernel a:0x800000000007d6c4-0x800000000007d73c]
  [OFW-4.93 lv2_kernel a:0x800000000007d7b0-0x800000000007d7cc].

## Where lv1 keeps the mappings

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. `cellgov_ps3_abi::lv1::device_bus`
holds each step with its address.

- The map function makes a 96-byte mapping object and fills it
  [OFW-4.93 lv1 a:0x225E60-0x225EB4]:
  - +0 holds the class of the TOC slot r2 - 28320, which holds
    0x35C538 [OFW-4.93 lv1 a:0x353198-0x35319C];
  - +8 holds the calling partition;
  - +16 and +40 hold the offset of the bus address in the region;
  - +48 holds the logical partition address and +56 the length;
  - the byte at +64 holds 1, and +72 holds the region;
  - +24 and +32 link the mapping to the one before and the one after it.
- It links the mapping into the region's list, whose head is at +72 of
  the region [OFW-4.93 lv1 a:0x225EB8-0x225F50].
- After the link, the device's map entry stores the logical partition
  address at +80 [OFW-4.93 lv1 a:0x227CCC].

## Files

| File | Rows | Holds |
|---|---|---|
| `dma_map.tsv` | 13 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields:

- `region_2_72`: the doubleword at +72 of region 2, the head of its
  mapping list.
- `mapping_1_class`, `mapping_1_partition`, `mapping_1_offset_16`,
  `mapping_1_previous`, `mapping_1_next`, `mapping_1_offset`,
  `mapping_1_lpar`, `mapping_1_length`, `mapping_1_64`,
  `mapping_1_region`, `mapping_1_lpar_80` and `mapping_1_88`: the
  doublewords at +0 to +88 of the first mapping of the list.

## The live mapping

- The list holds one mapping: its previous and next links are 0.
- The mapping has the class 0x35C538, the partition 2 of the game OS,
  and the region link 0x38B390, which is region 2.
- Its offset is 0, so its bus address is 0xC0000000. Its logical
  partition address is 0x800000, and its length is 0x100000, one page of
  the region.
- The byte at +64 is 1, and +80 holds the logical partition address. So
  the map entry ran to its end, and lv1 answered the kernel's call with
  0.
