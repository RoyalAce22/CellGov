# lv2_boot_spe_plug_connects

A read-only capture of the four connect steps that the 4.93 kernel runs
after the construct of its boot logical SPE. Each step sends lv1's
`connect_irq_plug_ext` (hypercall 12). The capture gives these values:
- the plug of each step on hardware thread 0;
- the proof that lv1 returned r3 = 0 for each step: for the SPE's irq
  outlets of classes 0 to 2, and for its event receive port.

CellGov's hypervisor answers those four steps in `cellgov_lv1`'s
`irq_plug_boot_outlet_tests`, with the plugs of this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-12, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads agree. It read the SPE object's words first, then the words at
  the addresses that they hold.
- It wrote nothing to lv1 or lv2, used no poke and installed nothing on
  the console.
- Only the 14 words below are committed.
- No console-identifying value is in this directory.

## Where the words are

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses.

- The boot SPE object is at 0x80000000003ddf48; see the capture
  `lv2_logical_spe_object`. Its +8 holds the spe id.
- After the create function, its caller runs a loop of four steps with
  the index 0 to 3
  [OFW-4.93 lv2_kernel a:0x8000000000034aac-0x8000000000034ae8]. Each
  step allocates a 56-byte object, and stores its address at
  +280 + 8 * index of the SPE object
  [OFW-4.93 lv2_kernel a:0x8000000000034adc].
- The step function stores the index at +8 of the 56-byte object
  [OFW-4.93 lv2_kernel a:0x800000000004f6d8]. It gives the kernel's
  connect step the id of the step, and the address of +24 of the object
  [OFW-4.93 lv2_kernel a:0x800000000004f6f0-0x800000000004f704]. The id
  is an outlet of classes 0 to 2 for the index 0 to 2, and the port at
  +88 for the index 3
  [OFW-4.93 lv2_kernel a:0x800000000004ee14-0x800000000004ee38].
- The connect step sends hypercall 12
  [OFW-4.93 lv2_kernel a:0x80000000000087f8]. It returns at once when r3
  is not 0 [OFW-4.93 lv2_kernel a:0x80000000000087fc-0x8000000000008808].
  When r3 is 0, it takes the plug's entry at +48 of the thread's record
  plus 88 * plug, and stores the entry's address at +24 of the 56-byte
  object
  [OFW-4.93 lv2_kernel a:0x800000000000880c-0x8000000000008814]
  [OFW-4.93 lv2_kernel a:0x800000000000883c].
- The record of hardware thread 0 is at 0x8000000000368f80; see the
  capture `lv2_logical_ppe_record`. Its +48 holds the address of its
  plug entries.

## Files

| File | Rows | Holds |
|---|---|---|
| `plug_connects.tsv` | 14 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields:
- `spe_id`: +8 of the SPE object;
- `plug_entries`: +48 of the record of thread 0;
- `step_0` to `step_3`: +280 to +304 of the SPE object;
- `index_0` to `index_3`: +8 of each 56-byte object, whose high word is
  the index;
- `entry_0` to `entry_3`: +24 of each 56-byte object.

## The live words

- The spe id is 11, as the capture `lv2_logical_spe_object` reads it.
  So the SPE object is the one that the image gives.
- The index of the objects at `step_0` to `step_3` is 0 to 3.
- Each entry is the plug entries' address plus 88 times a plug:
  - step 0, the class 0 outlet: plug 4;
  - step 1, the class 1 outlet: plug 5;
  - step 2, the class 2 outlet: plug 6;
  - step 3, the port: plug 7.
- The connect step computes and stores an entry only when lv1 returns
  r3 = 0. So lv1 connected each outlet and the port.
- The kernel takes the first free plug of the thread's map for each
  connect step; see the capture `lv2_irq_plug_map`. So plugs 0 to 3 of
  thread 0 were taken before these steps.
