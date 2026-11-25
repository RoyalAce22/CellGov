# lv2_repository_boot_nodes

A hardware probe. It reads back, from the running 4.93 kernel's memory,
the values the kernel read from the lv1 repository at boot. CellGov's
hypervisor serves these values to a 4.93 kernel boot from this capture.
No public source gives them for any firmware a kernel boot logs.

## Where the values sit

The stock 4.93 `lv2_kernel.self` reads its boot repository nodes in one
function and stores each value into one static record. The record lies in
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

The record is at 0x800000000037a5a0. The kernel finds it through the TOC
slot at r2 - 31808, and stores its address in the pointer at
0x800000000037a590. The kernel loads r2 from the doubleword at
0x8000000000003000.

## Readers of the second output

A repository read returns two values, in r4 and r5. The record holds only
the r4 value of each read, so the capture does not observe r5. A static
analysis of the stock 4.93 kernel (`cellgov dev disasm`) shows that no
boot reader reads r5 back:

- Every boot read goes through one leaf wrapper at 0x800000000000ce04.
  Its `sc` is at 0x800000000000ce3c. On success it stores r4 through its
  third argument and r5 through its fourth.
- Each node has one leaf accessor that calls the wrapper. The accessor
  passes a stack slot as the fourth argument and does not load that slot
  again. The wrapper makes no call before its `sc`, so the link register
  at the `sc` is the accessor's return address.

| Node | Return address of the wrapper call |
|---|---|
| lpar 1 `be.0.clock` | 0x800000000000c85c |
| lpar 1 `plat.id` | 0x800000000000c8d0 |
| lpar 2 `bi.rgntotal` | 0x800000000000c94c |
| lpar 2 `bi.pu.<id>.rm_addr` | 0x800000000000ca0c |
| lpar 2 `bi.pu.<id>.rm_size` | 0x800000000000ca38 |
| lpar 2 `bi.pu` at index 0 | 0x800000000000caf8 |
| lpar 2 `bi.pun` | 0x800000000000cba4 |

No other `bl` in the kernel's text calls the wrapper. A CellGov boot of
the stock 4.93 kernel makes each read from the site in its row
(`docs/lv2/tables/repository_read.tsv`): a read from another site would
stop the boot at a refusal. CellGov's
hypervisor puts 0 in r5 for a node from this capture only when the read
comes from the `sc` and return address of that node's row. It refuses a
read of the node from any other site.

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
| 0x800000000037a630 | 0x0000000007900000 | 0x0f9a0000 - 0x08000000 - 0x000a0000 |
| 0x800000000037a640 | 0x00000000000a0000 | 0x0f9a0000 & 0xfffff |
| 0x800000000037a648 | 0x436f6b4731310000 | `plat.id` = "CokG11" |
| 0x800000000037a650 | 0x0000000004c1a6c0 | `be.0.clock` = 79 800 000 |

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
