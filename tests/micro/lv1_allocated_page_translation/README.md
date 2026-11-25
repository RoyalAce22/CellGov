# lv1_allocated_page_translation

A read-only capture of the code that lv1's `write_htab_entry`
(hypercall 1) and `allocate_memory` (hypercall 0) run on the reference
console, at firmware 4.93, for a page in a region that `allocate_memory`
created. It gives these values:
- the TOC slot, the class entry and the function descriptor that lead
  from `write_htab_entry` to the method that translates a page of an
  allocated region;
- for each code range that `cellgov_ps3_abi::lv1::memory` and
  `cellgov_lv1`'s page table code cite for the two calls, whether the
  live words are the words of the shipped image.

`cellgov_ps3_abi`'s `memory_allocated_page_console_tests` holds each
lv1 citation of `WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3` to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-12, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads of each word agree.
- It wrote nothing to lv1 or lv2, used no poke and installed nothing on
  the console.
- The code words are not committed. `code.tsv` records only whether
  they equal the shipped image.
- No console-identifying value is in this directory.

## Where the words are

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses.

- The region that `allocate_memory` creates has its class in the TOC
  slot at r2 - 8104 [OFW-4.93 lv1 a:0x2DEAC4-0x2DEACC]
  [OFW-4.93 lv1 a:0x358090].
- `write_htab_entry` calls the method at +24 of that class
  [OFW-4.93 lv1 a:0x3621E8]. The entry holds the method's descriptor
  [OFW-4.93 lv1 a:0x347BC8].
- `write_htab_entry` searches the entry's group for a duplicate
  [OFW-4.93 lv1 a:0x2D2F20-0x2D3038] before it translates the page
  [OFW-4.93 lv1 a:0x2D30C4].

## Files

| File | Rows | Holds |
|---|---|---|
| `words.tsv` | 3 | for each data word: its address, its role, the number of reads, whether they agree, and its value |
| `code.tsv` | 11 | for each code range: its first and last byte, what it holds, the doublewords read, whether each word's two reads agree, and how it compares with the stock image |

The roles in `words.tsv`:
- `toc_slot_allocated_region_class`: the TOC slot at r2 - 8104;
- `allocated_region_class_entry_24`: the entry at +24 of that class;
- `translate_method_descriptor`: the descriptor in that entry, whose
  first doubleword is the method's code address.

## The live words

- The TOC slot holds the class at 0x3621D0, its entry at +24 holds the
  descriptor at 0x347BC8, and the descriptor names the method at
  0x2DDF98. Each is the value of the image.
- Each of the 125 code doublewords is the doubleword of the shipped
  image. The ranges hold the allocation of the region's pages and their
  zero fill, the region's constructor stores, the duplicate search and
  the translation call of `write_htab_entry`, the method that checks the
  region's owner and the page size and computes the real address, and
  the handler's conversion of the method's code to r3.

## What the capture does not settle

The capture shows the code that the console runs. It does not run
either call. A user process cannot make them: a problem-state `sc 1` to
a number below 256 gets r3 = 0x00000000FFFFFFFD from the vector (the
captures `lv1_problem_state_sc` and `lv1_hypercall_table`). So the r3 of
an entry whose page is larger than the region's pages, the order of the
duplicate search and the translation, and the zero fill of the pages
come from the code alone. No capture observes them.
