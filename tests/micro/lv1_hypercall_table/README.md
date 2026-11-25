# lv1_hypercall_table

A read-only capture of lv1's live hypercall table on the reference
console, at firmware 4.93. It says, for each hypercall number below 256,
whether the running lv1 has a handler for it. CellGov's hypervisor reads
this capture, through `cellgov_ps3_abi::lv1::table`, to answer a number
lv1 marks invalid.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- webMAN's `peek.lv1` HTTP command returns 0x200 bytes of lv1 memory at
  an address. The capture used that command only. It wrote nothing to
  lv1 or lv2, used no poke and installed nothing on the console.
- Each page was read twice, on 2026-11-10. The two reads of each page
  are identical.
- The raw HTTP pages are not committed, because they carry webMAN's page
  around the bytes. The TSV files hold the doublewords parsed from them.
- No console-identifying value is in this directory.

## Where the table is

lv1 builds its hypercall table at run time. The shipped 4.93 `lv1.self`,
decrypted offline and read with `cellgov dev disasm`, gives the
addresses:

- `set_lv1_hvcall_table` [OFW-4.93 lv1 a:0x2C8990] stores the entry
  count 256 (`li r0, 256`) and the table base, from the TOC slot
  [OFW-4.93 lv1 a:0x357AA0].
- The Invalid HV Call function is at [OFW-4.93 lv1 a:0x2C8B80]. It
  returns r3 = 0x00000000FFFFFFEC and nothing else.
- The table is 256 doublewords at [OFW-4.93 lv1 a:0x372D08]. Each one is
  the code address of the number's handler, and its low bit is the
  problem-state-callable flag. The memory call table, 256 doublewords at
  [OFW-4.93 lv1 a:0x373508], follows it.

## Files

| File | Rows | Holds |
|---|---|---|
| `hypercall_table.tsv` | 256 | each number, its table doubleword, and `handler` or `invalid` (the doubleword is 0x2C8B80) |
| `memory_call_table.tsv` | 256 | each number's memory call descriptor address, or 0 |
| `toc_slots.tsv` | 4 | the TOC slots that `set_lv1_hvcall_table` and the invalid entries read |
| `pages.tsv` | 15 | each page read, whether its two reads agree, and how it compares with the stock image |

## The live table

- 151 numbers have a handler. 105 numbers hold the Invalid HV Call
  function.
- No entry has its low bit set. So at 4.93 every problem-state `sc 1`
  to a number below 256 gets r3 = 0x00000000FFFFFFFD from the vector.
- 30 numbers route to the memory call thunk at
  [OFW-4.93 lv1 a:0x2C89CC]: 0, 6, 13, 15,
  90 to 93, 105 to 112, 116 to 119, 122 to 126, 135, 136, 182, 183 and
  231. The memory call table holds a descriptor for exactly these 30
  numbers.
- Every handler address lies in the text segment of the stock image
  [OFW-4.93 lv1 a:0x200000-0x3C2C00]. Every memory call descriptor is
  stock data
  whose code address lies in that segment.

lv1 fills both tables at run time, and the stock image holds zeros
there. So the image cannot show whether stock lv1 fills exactly this
set. The capture shows only that the live table names stock code.

## Comparison with the stock image

The code pages and the TOC page are identical to the shipped 4.93 image,
except one word. At 0x1028, on the vector's return path after the call
[OFW-4.93 lv1 a:0x1024], the stock image holds `addi r1, r1, 112`
[OFW-4.93 lv1 a:0x1028] and the console holds 0xFFFF0070. The console
runs custom firmware, which can patch lv1. This difference is recorded
and left unresolved.

The table pages are zero in the stock image. Every nonzero word there is
run-time data.
