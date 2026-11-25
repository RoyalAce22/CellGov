# lv2_logical_spe_object

A read-only capture of the 4.93 kernel's boot-time logical SPE object on
the reference console. It gives these values:
- the spe id and the shadow address that lv1's `construct_logical_spe`
  (hypercall 57) gave the kernel's boot call;
- the outlet id of each interrupt class that lv1's `get_spe_irq_outlet`
  (hypercall 78) gave the kernel for that SPE.

CellGov's hypervisor answers those calls with these values in
`cellgov_lv1`'s `construct_capture_tests` and
`irq_outlet_capture_tests`.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- `cellgov ps3 peek --space lv2 --width 8` read the doublewords below
  through webMAN's `peek.lv2` command, after the console had booted to
  the XMB. It read each page twice, and the two reads agree:
  - on 2026-11-11, the object, the spe id and the shadow;
  - on 2026-11-12, the three outlets, with the spe id again, which read
    11 as before.

  It wrote nothing to lv1 or lv2, used no poke and installed nothing on
  the console.
- Only the six words below are committed.
- No console-identifying value is in this directory.

## Where the object is

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses. The kernel's r2 is
0x800000000034fbb0 [OFW-4.93 lv2_kernel a:0x8000000000003000].

- The TOC slot r2 - 28872 [OFW-4.93 lv2_kernel a:0x8000000000348ae8]
  holds 0x80000000003ddf48, the address of the object. The address is in
  the zero-filled part of the kernel's data segment: the segment's file
  image ends at 0x8000000000368f60. So each field below is 0 when the
  kernel starts.
- The function at 0x8000000000034b50 loads the slot
  [OFW-4.93 lv2_kernel a:0x8000000000034b5c]. Its constructor call
  [OFW-4.93 lv2_kernel a:0x8000000000034b2c] does not write +8 or +16.
  It then calls the create function 0x800000000004f13c on the object
  [OFW-4.93 lv2_kernel a:0x8000000000034b80]
  [OFW-4.93 lv2_kernel a:0x8000000000034aa8].
- The create function sets r3 to r7 to 12, r8 to the vas id and r9 to 1
  [OFW-4.93 lv2_kernel a:0x800000000004f158-0x800000000004f184]. It sends
  hypercall 57 [OFW-4.93 lv2_kernel a:0x800000000004f188]. On r3 = 0 it
  stores r8 at +16 and r9 at +8
  [OFW-4.93 lv2_kernel a:0x800000000004f18c-0x800000000004f198]. It
  uses no other output of the call.
- It then sends hypercall 78 for each class from 0 to 2, with the spe id
  from +8 in r3 and the class in r4. On r3 = 0 it stores r4 at +32,
  +40 or +48 for class 0, 1 or 2
  [OFW-4.93 lv2_kernel a:0x800000000004f1e4-0x800000000004f210]. It uses
  no other output of the call.

## Files

| File | Rows | Holds |
|---|---|---|
| `spe_object.tsv` | 6 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields are `object` (the TOC slot), `spe_id` (the object's +8),
`shadow` (the object's +16) and `outlet_0` to `outlet_2` (the object's
+32, +40 and +48).

## The live object

- The TOC slot holds its stock value, so the two object words are at the
  addresses that the image gives.
- The spe id is 11. It is not 0, so the call returned r3 = 0.
- The shadow address 0x0000300000006000 is the 4 KB region of the
  partition's region counter value 6: the size exponent 12 is in bits 42
  and up, and the counter is in bits 12 to 41. See
  `cellgov_ps3_abi::lv1::memory`.
- lv1 gives the SPE's 512 KB area the counter value before the shadow's;
  see `cellgov_ps3_abi::lv1::spe`. So the area took the value 5.
- In CellGov's boot of the same kernel, these region-creating calls come
  before this one:
  - the two boot allocations, which take the values 2 and 3, as the
    capture `lv2_repository_boot_nodes` reads them;
  - the page table mapping, which takes the value 4.

  So the call takes the values 5 and 6, as it did on the console.
- The outlet ids are 13, 14 and 15. None is 0, so each call returned
  r3 = 0. lv1 makes the three outlets in hypercall 57, one per class in
  class order, and each takes the first free id of the partition's
  object table; see `cellgov_ps3_abi::lv1::spe`.
