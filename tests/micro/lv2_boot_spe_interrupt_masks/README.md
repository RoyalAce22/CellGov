# lv2_boot_spe_interrupt_masks

A read-only capture of the interrupt masks that the 4.93 kernel keeps
for its boot logical SPE. The kernel keeps the mask that it last passed
to lv1's `set_spe_interrupt_mask` (hypercall 61) for each interrupt
class. The capture gives these values:
- the spe id of the boot SPE;
- the last mask of each class from 0 to 2.

CellGov's hypervisor answers the kernel's boot calls of hypercall 61 in
`cellgov_lv1`'s `interrupt_mask_boot_tests`, and the masks that it
keeps there are the masks of this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-12, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. It read the page twice, and the two
  reads agree.
- It wrote nothing to lv1 or lv2, used no poke and installed nothing on
  the console.
- Only the four words below are committed.
- No console-identifying value is in this directory.

## Where the words are

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses.

- The boot SPE object is at 0x80000000003ddf48; see the capture
  `lv2_logical_spe_object`. Its +8 holds the spe id.
- The kernel's mask function sends hypercall 61 with the spe id from +8
  of the object in r3, the class in r4 and the mask in r5
  [OFW-4.93 lv2_kernel a:0x8000000000034c34-0x8000000000034c48]. It then
  stores the mask at +56 + 8 * class of the object, and does not read
  r3 [OFW-4.93 lv2_kernel a:0x8000000000034c4c-0x8000000000034c60].
- After the create function, its caller runs a loop of four steps with
  the index 0 to 3
  [OFW-4.93 lv2_kernel a:0x8000000000034aac-0x8000000000034ae8]. The
  step for the index 0 to 2 calls the mask function for the class that
  the index names twice:
  - before its connect step, with the mask 0;
  - after its connect step, with the mask at +16 of its 56-byte object
    [OFW-4.93 lv2_kernel a:0x800000000004f794-0x800000000004f7a8].
- The steps store the masks 7, 3 and 7 at +16 for the index 0, 1 and 2
  [OFW-4.93 lv2_kernel a:0x800000000004f81c]
  [OFW-4.93 lv2_kernel a:0x800000000004f73c]
  [OFW-4.93 lv2_kernel a:0x800000000004f7e4]. The step for the index 3
  makes no call
  [OFW-4.93 lv2_kernel a:0x800000000004f83c-0x800000000004f854].

## Files

| File | Rows | Holds |
|---|---|---|
| `interrupt_masks.tsv` | 4 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields:
- `spe_id`: +8 of the SPE object;
- `mask_0` to `mask_2`: +56, +64 and +72 of the SPE object, the last mask
  of classes 0 to 2.

## The live words

- The spe id is 11, as the capture `lv2_logical_spe_object` reads it.
  So the SPE object is the one that the image gives.
- The masks are 7, 3 and 7 for classes 0 to 2. These are the masks of the
  second call of each step. So no later call of the mask function
  changed them before the read.
- The words record the masks that the kernel passed. lv1 keeps its own
  copy in hypervisor memory; see `cellgov_ps3_abi::lv1::spe`.
