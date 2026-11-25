# lv2_repository_boot_nodes

A hardware probe. It reads back, from the running 4.93 kernel's memory,
the values the kernel read from the lv1 repository at boot. CellGov's
hypervisor serves these values to a 4.93 kernel boot from this capture.
No public source gives them for any firmware a kernel boot logs.

## Where the values sit

The shipped 4.93 `lv2_kernel.self` reads its boot repository nodes in one
function [OFW-4.93 lv2_kernel a:0x800000000000cbd4-0x800000000000cd4c]
and stores each value into one static record. The record lies in
the zero-fill part of the data segment, so every value in it is written at
run time. The offsets come from that function's code:

| Offset | Value | How the kernel gets it |
|---|---|---|
| +0x08 | the partition id | hypercall 74 |
| +0x10 | lpar 2 `bi.pun` | hypercall 91 |
| +0x18 | lpar 2 `bi.pu` at index 0 | hypercall 91 |
| +0x30 | lpar 2 `bi.pu.<id>.rm_addr` | hypercall 91 |
| +0x38 | lpar 2 `bi.pu.<id>.rm_size` | hypercall 91 |
| +0x68 | lpar 2 `bi.rgntotal` | hypercall 91 |
| +0x70 | 3 when the low 20 bits of `bi.rgntotal` are not 0, else 2 | derived |
| +0x80 | `rm_size` again | copied |
| +0x90 | `bi.rgntotal` - `rm_size` - (`bi.rgntotal` & 0xfffff) | derived |
| +0xa0 | `bi.rgntotal` & 0xfffff, when it is not 0 | derived |
| +0xa8 | lpar 1 `plat.id` | hypercall 91 |
| +0xb0 | lpar 1 `be.0.clock` | hypercall 91 |
| +0xb8 | BP_VR, the processor version register (a word) | `mfspr` of SPR 1022 |

The record is at 0x800000000037a5a0. The kernel finds it through the TOC
slot at r2 - 31808, and stores its address in the pointer at
[OFW-4.93 lv2_kernel a:0x800000000037a590]. The kernel loads r2 from the
doubleword at [OFW-4.93 lv2_kernel a:0x8000000000003000].

## Readers of the second output

A repository read returns two values, in r4 and r5. The record holds only
the r4 value of each read, so the capture does not observe r5. A static
analysis of the shipped 4.93 kernel (`cellgov dev disasm`) shows that no
boot reader reads r5 back:

- Every boot read goes through one leaf wrapper at
  [OFW-4.93 lv2_kernel a:0x800000000000ce04]. Its `sc` is at
  [OFW-4.93 lv2_kernel a:0x800000000000ce3c]. On success it stores r4
  through its third argument and r5 through its fourth.
- Each node has one leaf accessor that calls the wrapper. The accessor
  passes a stack slot as the fourth argument and does not load that slot
  again. The wrapper makes no call before its `sc`, so the link register
  at the `sc` is the accessor's return address.

| Node | Return address of the wrapper call |
|---|---|
| lpar 1 `be.0.clock` | [OFW-4.93 lv2_kernel a:0x800000000000c85c] |
| lpar 1 `plat.id` | [OFW-4.93 lv2_kernel a:0x800000000000c8d0] |
| lpar 2 `bi.rgntotal` | [OFW-4.93 lv2_kernel a:0x800000000000c94c] |
| lpar 2 `bi.pu.<id>.rm_addr` | [OFW-4.93 lv2_kernel a:0x800000000000ca0c] |
| lpar 2 `bi.pu.<id>.rm_size` | [OFW-4.93 lv2_kernel a:0x800000000000ca38] |
| lpar 2 `bi.pu` at index 0 | [OFW-4.93 lv2_kernel a:0x800000000000caf8] |
| lpar 2 `bi.pun` | [OFW-4.93 lv2_kernel a:0x800000000000cba4] |

No other `bl` in the kernel's text calls the wrapper. A CellGov boot of
the shipped 4.93 kernel makes each read from the site in its row
(`docs/lv2/tables/repository_read.tsv`): a read from another site would
stop the boot at a refusal. CellGov's
hypervisor puts 0 in r5 for a node from this capture only when the read
comes from the `sc` and return address of that node's row. It refuses a
read of the node from any other site.

## The processor version word

The function that fills the record ends with a read of BP_VR, the
CBEA-compliant processor version register. BP_VR is a 32-bit register,
so `mfspr` puts it in the low word of r0. The code at
[OFW-4.93 lv2_kernel a:0x800000000000cd18-0x800000000000cd2c] is:

```text
mfspr r0, 1022
stw   r0, 112(r1)
isync
lwz   r0, 112(r1)
stw   r0, 184(r31)
```

r31 holds the record address, so the word at +0xb8 is the whole register,
with no bit lost. The word at +0xbc stays zero fill.

The read is not a no-op, and it does not return 0. Before the `mfspr`, r0
holds the return address of the `be.0.clock` accessor, whose low word is
0x0000cd14. The capture reads 0x00002100.

No other code writes the word:

- One accessor at [OFW-4.93 lv2_kernel a:0x800000000000cd80] runs the
  fill function once, behind a byte flag. It is the only code that loads
  the record's TOC slot.
- The record's pointer has one more reader, the getter at
  [OFW-4.93 lv2_kernel a:0x800000000000c7f4]. The getter's only caller
  reads the record and does not write it.
- No data word holds the function descriptor of the getter, the accessor
  or the fill function.
- The record's two virtual functions, at
  [OFW-4.93 lv2_kernel a:0x800000000000cdd4] and
  [OFW-4.93 lv2_kernel a:0x800000000000cde0], write only its first
  doubleword.

CellGov's PPU answers a supervisor-state read of BP_VR with this value
(`cellgov_ps3_abi::hw::ppu::CELL_BP_VR`).

## The memory allocations

After the fill function, the kernel makes two lv1 `allocate_memory`
calls (hypercall 0) in the function at
[OFW-4.93 lv2_kernel a:0x8000000000265368]. Each call takes its size from
the record and stores the address that lv1 returns in r4 back into the
record:

| `sc` | r3, the size | r4, the page size exponent | r4 out stored at |
|---|---|---|---|
| [OFW-4.93 lv2_kernel a:0x80000000002653c0] | +0x90 | 20 | +0x88 |
| [OFW-4.93 lv2_kernel a:0x800000000026541c] | +0xa0, low 16 bits cleared | 16 | +0x98 |

r5 and r6 are 0 at both calls. The kernel takes the first size from the
record when +0x70 is greater than 1, and the second when +0x70 is
greater than 2; +0x70 is 3 here. It skips a call whose size is 0. It
stores r4 only when r3 is 0, and calls its panic routine on any other
status. Neither path reads r5, the memory
unit id: the success path copies r3 over r5 before any read of it.

The capture reads 0x00006c0010000000 at +0x88 and 0x0000500000300000 at
+0x98. A logical partition address holds the region's size exponent at
bit 42 and the partition's region counter above the exponent
(psdevwiki "Hypervisor Reverse Engineering", "Encoding LPAR Memory
Region Start Addresses and Sizes"):

| Size | Exponent | Address | Counter |
|---|---|---|---|
| 0x07900000 | 27 | 0x00006c0010000000 | 2 |
| 0x000a0000 | 20 | 0x0000500000300000 | 3 |

Neither size is a power of two, so the exponent is the size's log2
rounded up. The counter takes 2 at the first call and 3 at the second.
CellGov's hypervisor answers the kernel's allocations from this
(`cellgov_ps3_abi::lv1::memory`).

The two sizes add up to 0x079a0000, which is `bi.rgntotal` less
`rm_size`. Linux `ps3_mm_init` asks lv1 for the same amount: the region
total less the real mode size.

## Method

- Firmware 4.93, the `cech20-cex-493` reference profile; captured
  2026-11-10 (the provenance records the time).
- The probe reads each word through the CFW's lv2 peek, syscall 6. It
  reads no kernel code and writes no kernel memory.
- A second, independent read used webMAN's `peek.lv2` page on the same
  console on the same day. It returned the same doubleword at each of
  the 25 addresses the two reads share (0x800000000037a590 to
  0x800000000037a658), and the same 0x200 bytes on two requests.
- The same webMAN peek showed that the running kernel's code is the stock
  4.93 code at the addresses the layout above depends on. Three 0x200-byte
  pages matched the decrypted stock kernel byte for byte: the function
  that fills the record, the repository read wrapper and its key builders,
  and the TOC page that holds every slot those functions use.

## The values

| Address | Doubleword | Meaning |
|---|---|---|
| 0x8000000000003000 | 0x800000000034fbb0 | r2, as in the stock kernel |
| 0x8000000000347f70 | 0x800000000037a5a0 | the TOC slot: the record |
| 0x800000000037a590 | 0x800000000037a5a0 | the pointer: the record |
| 0x800000000037a5a8 | 0x0000000000000002 | partition id 2 |
| 0x800000000037a5b0 | 0x0000000000000001 | `bi.pun` = 1 |
| 0x800000000037a5b8 | 0x0000000000000001 | `bi.pu` = 1 |
| 0x800000000037a5d0 | 0x0000000000000000 | `rm_addr` = 0 |
| 0x800000000037a5d8 | 0x0000000008000000 | `rm_size` = 128 MiB |
| 0x800000000037a608 | 0x000000000f9a0000 | `bi.rgntotal` |
| 0x800000000037a610 | 0x0000000000000003 | low 20 bits not 0 |
| 0x800000000037a620 | 0x0000000008000000 | `rm_size` again |
| 0x800000000037a628 | 0x00006c0010000000 | the first allocation's address |
| 0x800000000037a630 | 0x0000000007900000 | 0x0f9a0000 - 0x08000000 - 0x000a0000 |
| 0x800000000037a638 | 0x0000500000300000 | the second allocation's address |
| 0x800000000037a640 | 0x00000000000a0000 | 0x0f9a0000 & 0xfffff |
| 0x800000000037a648 | 0x436f6b4731310000 | `plat.id` = "CokG11" |
| 0x800000000037a650 | 0x0000000004c1a6c0 | `be.0.clock` = 79 800 000 |
| 0x800000000037a658 | 0x0000210000000000 | BP_VR = 0x00002100 in the high word |

`cgov_frame.bin` holds all 27 reads as (address, doubleword) pairs.

## Cross-checks

- The three derived words agree with the read values: +0x90 and +0xa0
  are what the kernel's code computes from `bi.rgntotal` and `rm_size`,
  and +0x70 is the state for a nonzero remainder.
- `be.0.clock` equals the Cell time base frequency CellGov already
  carries (`cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ`).
- `bi.pun`, `bi.pu` and `rm_addr` equal the values of the psdevwiki
  repository dump, which CellGov serves to the older firmware. `rm_size`
  does not: the dump gives 16 MiB, and this console gives 128 MiB at
  4.93.
- The two allocation addresses decode by the psdevwiki address encoding
  to the sizes the kernel passed, and to consecutive counter values. The
  psdevwiki list of the 3.41 GameOS regions holds a region of the same
  kind: 0xE900000 bytes in 2^20-byte pages with flags 0, at
  0x700020000000, which is counter 2 with the exponent rounded up to 28.
  psdevwiki lists the second address, 0x500000300000 with 0xA0000 bytes,
  for 3.41 too.
- A webMAN `peek.lv2` of 0x800000000037a580 on 2026-11-10, after the
  capture, returned the same two allocation addresses and the same
  record words.
