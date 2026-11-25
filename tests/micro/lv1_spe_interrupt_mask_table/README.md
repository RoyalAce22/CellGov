# lv1_spe_interrupt_mask_table

A read-only capture of the code and data that lv1's
`set_spe_interrupt_mask` (hypercall 61) runs on the reference console,
at firmware 4.93. It gives these values:
- the table of the mask bits that the call keeps for each interrupt
  class, and the TOC slot that points to it;
- the TOC slots and the function descriptors that lead from the call to
  the method that ANDs the mask and to the method that stores it;
- for each code range that `cellgov_ps3_abi::lv1::spe` cites for the
  call, whether the live words are the words of the shipped image.

`cellgov_ps3_abi`'s `spe_interrupt_mask_console_tests` holds
`SPE_INTERRUPT_CLASS_MASKS` to the table of this capture.

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
`cellgov dev disasm`, gives the addresses. The TOC base of the call's
functions is 0x35A038, so the slot at r2 - 5016 is at 0x358CA0, the
slot at r2 - 4704 at 0x358DD8, the slot at r2 - 4616 at 0x358E30 and the
slot at r2 - 344 at 0x359EE0.

- The method of the call ANDs the mask with the class's entry of the
  table [OFW-4.93 lv1 a:0x2FA24C-0x2FA25C] in the TOC slot at r2 - 5016
  [OFW-4.93 lv1 a:0x358CA0]. The table is three doublewords
  [OFW-4.93 lv1 a:0x329A98-0x329AAC].
- The worker gives the mask to the method in the TOC slot at r2 - 4616
  [OFW-4.93 lv1 a:0x2FE530]. That slot holds the method's descriptor
  [OFW-4.93 lv1 a:0x34AA78].
- The worker checks the object's class against the TOC slot at r2 - 4704
  [OFW-4.93 lv1 a:0x2FE370-0x2FE494].
- The interrupt object's class is in the TOC slot at r2 - 344
  [OFW-4.93 lv1 a:0x3192F0-0x3192F8]. The class's entry at +48
  [OFW-4.93 lv1 a:0x363D28] holds the descriptor
  [OFW-4.93 lv1 a:0x3519D8] of the method that stores the mask
  [OFW-4.93 lv1 a:0x3198F4].

## Files

| File | Rows | Holds |
|---|---|---|
| `words.tsv` | 10 | for each data word: its address, its role, the number of reads, whether they agree, and its value |
| `code.tsv` | 11 | for each code range: its first and last byte, what it holds, the doublewords read, whether each word's two reads agree, and how it compares with the stock image |

The roles in `words.tsv`:
- `class_mask_0` to `class_mask_2`: the table's entry of classes 0 to 2;
- `toc_slot_class_masks`: the TOC slot at r2 - 5016;
- `toc_slot_spe_class`: the TOC slot at r2 - 4704;
- `toc_slot_mask_entry`: the TOC slot at r2 - 4616;
- `mask_entry_descriptor`: the descriptor in that slot, whose first
  doubleword is the method's code address;
- `toc_slot_interrupt_object_class`: the TOC slot at r2 - 344;
- `interrupt_object_class_entry_48`: the entry at +48 of that class;
- `mask_method_descriptor`: the descriptor in that entry, whose first
  doubleword is the code address of the method that stores the mask.

## The live words

- The table holds 0xFFFFFFFEFFFFFFF7 for class 0 and
  0xFFFFFFFFFFFFFFFF for classes 1 and 2, and the TOC slot points to it.
  So the running lv1 clears bits 0x0000000100000000 and 0x8 of a class 0
  mask and keeps each bit of a class 1 or 2 mask.
- Each slot and descriptor leads to the address that the image gives.
- Each of the 219 doublewords is the doubleword of the shipped image.
  The one word in which the console's lv1 differs from the image at the
  vector's return path (see the capture `lv1_hypercall_table`) is
  outside these ranges.

## What the capture does not settle

The capture shows the code and the table that the console runs. It does
not run the call. A user process cannot make it: a problem-state `sc 1`
to a number below 256 gets r3 = 0x00000000FFFFFFFD from the vector (the
captures `lv1_problem_state_sc` and `lv1_hypercall_table`). The kernel's
boot masks (the capture `lv2_boot_spe_interrupt_masks`) set no bit that
the table clears. So the answer of the call to a mask with those bits
set comes from the code alone. No capture observes it.
