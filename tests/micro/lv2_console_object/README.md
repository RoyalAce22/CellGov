# lv2_console_object

A read-only capture of the 4.93 kernel's logical console object on the
reference console. It gives the console id that the kernel passes to
lv1's logical console calls, and the state that the kernel leaves after
its boot-time construct, destruct and construct. CellGov's hypervisor
answers those calls on this id in `cellgov_lv1`'s
`console_lv2_console_object_tests`.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- webMAN's `peek.lv2` HTTP command returns 0x200 bytes of kernel memory
  at an address. The capture used that command only, at
  0x800000000037a6c0, after the console had booted to the XMB. It wrote
  nothing to lv1 or lv2, used no poke and installed nothing on the
  console.
- The page was read twice, on 2026-11-11. The two reads are identical.
- Only the four words below are committed. The rest of the page is
  kernel data that CellGov does not use, and the raw HTTP pages are
  deleted.
- No console-identifying value is in this directory.

## Where the object is

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses. The kernel's r2 is
0x800000000034fbb0 [OFW-4.93 lv2_kernel a:0x8000000000003000].

- The TOC slot r2 - 31496 [OFW-4.93 lv2_kernel a:0x80000000003480a8]
  holds 0x800000000037a6c8, the address of a flag byte. The TOC slot
  r2 - 31488 [OFW-4.93 lv2_kernel a:0x80000000003480b0] holds
  0x800000000037a6d0, the array of 312-byte console objects
  [OFW-4.93 lv2_kernel a:0x8000000000010520-0x800000000001052c].
- Both addresses are in the zero-filled part of the kernel's data
  segment: its file image ends at 0x8000000000368f60. So the flag, the
  object and its state are 0 when the kernel starts.
- An object's console state is at object + 176
  [OFW-4.93 lv2_kernel a:0x8000000000010994]: a state word at +0 and
  the console id at +8.

## What the kernel does at boot

- The function at 0x80000000002659d0 sets the flag to 1
  [OFW-4.93 lv2_kernel a:0x80000000002659d0-0x80000000002659f0]. The
  only store to the flag is in 0x8000000000010ca4
  [OFW-4.93 lv2_kernel a:0x8000000000010cbc].
- 0x8000000000010ca4 then constructs object 0. The console id is the
  doubleword at object + 144 plus 1
  [OFW-4.93 lv2_kernel a:0x80000000000109c0-0x80000000000109d0]. The
  construct wrapper sends hypercall 105 only when the state word is 0.
  On r3 = 0 it stores the id at state + 8 and 1 in the state word
  [OFW-4.93 lv2_kernel a:0x800000000000fc7c-0x800000000000fc84].
- At 0x8000000000265adc the same function calls 0x8000000000010974 on object 0
  [OFW-4.93 lv2_kernel a:0x8000000000265ad8-0x8000000000265adc]. With
  the flag set, that goes to the destruct wrapper
  [OFW-4.93 lv2_kernel a:0x8000000000010994-0x80000000000109a0]. When
  the state word is 1, the wrapper sends hypercall 106 with the id from
  state + 8 in r3 [OFW-4.93 lv2_kernel a:0x80000000000104e0-0x80000000000104e8].
  On r3 = 0 it stores 0 in the state word
  [OFW-4.93 lv2_kernel a:0x80000000000104f4]. On any other r3 it
  branches to itself [OFW-4.93 lv2_kernel a:0x80000000000104fc], and
  the boot stops there.
- At 0x8000000000265ae4 the function clears the flag and constructs
  object 0 again. On a nonzero r3 it branches to itself
  [OFW-4.93 lv2_kernel a:0x8000000000010ce0-0x8000000000010ce8].

## Files

| File | Rows | Holds |
|---|---|---|
| `console_object.tsv` | 4 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields are `flag` (the flag byte), `id_base` (the doubleword at
object 0 + 144), `state` (object 0's state word) and `id` (object 0's
console id).

## The live object

- The console id is 1, `id_base` plus 1. The kernel stored it on a
  construct, and its destruct sends it.
- The state word is 1 and the flag is 0. The console booted to the XMB,
  so the boot passed the destruct and the second construct, and both
  hypercalls gave r3 = 0. With the console gate at 2
  (`tests/micro/lv1_console_gate`), lv1 answers the first construct
  with r3 = 0 too, so the destruct ran.
