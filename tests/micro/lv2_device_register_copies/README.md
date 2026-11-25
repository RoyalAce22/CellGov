# lv2_device_register_copies

A read-only capture, in lv2 on the reference console, of the copies
that the running 4.93 kernel keeps of two device registers of devices
1 and 2 of bus 1. The kernel reads the registers once, from the MMIO
region of each device at the bus addresses 0x3810000 and 0x3820000,
and keeps what it derives from them in its controller object. The
capture reads those copies. It never reads a device register.

CellGov answers the kernel's two reads of the registers of device 1
from this capture (`cellgov_ps3_abi::hw::device_register::DEVICE_REGISTER_READS`).
The test `device_register_console_tests` holds those facts to the
capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the reads
returned, as `cellgov ps3 peek` wrote them.

## What the kernel reads

The function at [OFW-4.93 lv2_kernel a:0x8000000000245818] receives a
controller object in r3. It finds the effective address of its MMIO
region as `*(*(*(r3+40)+80)+56)`, and reads two words there:

| Instruction | Read | What the kernel keeps |
|---|---|---|
| [OFW-4.93 lv2_kernel a:0x800000000024590C] `lwz r0,0(r11)` | the word at +0 | the low byte, added to the region's address, as the doubleword at +128 |
| [OFW-4.93 lv2_kernel a:0x8000000000245910] `lwz r9,4(r11)` | the word at +4 | the low 4 bits, as the byte at +48 |

The next two instructions keep only those bits:
`clrldi r0,r0,56` at [OFW-4.93 lv2_kernel a:0x8000000000245914] and
`clrlwi r9,r9,28` at [OFW-4.93 lv2_kernel a:0x8000000000245918]. No
instruction between the loads and the masks reads r0 or r9. The
kernel returns 0x8001002d when the 4 bits are 0. Otherwise it calls
[OFW-4.93 lv2_kernel a:0x80000000002422B4], whose first access is a
store of 0 to the word at +8 of the doubleword at +128
[OFW-4.93 lv2_kernel a:0x80000000002422DC].

So the copies give the low byte of word +0 and the low 4 bits of word
+4, and nothing more of either register. The capture gives no other
bit of the two words.

No other code writes the two copies:

- In the code from 0x800000000023C000 to 0x800000000024FFFF, the only
  stores to +128 of an object are the constructor's store of 0
  [OFW-4.93 lv2_kernel a:0x8000000000243D6C] and the function's store
  [OFW-4.93 lv2_kernel a:0x8000000000245924]. Every other store to
  +128 there is to the stack.
- The only byte stores to +48 there are these:
  - the function's store [OFW-4.93 lv2_kernel a:0x8000000000245928];
  - two byte-by-byte copies of a record
    [OFW-4.93 lv2_kernel a:0x8000000000247338]
    [OFW-4.93 lv2_kernel a:0x80000000002478D8];
  - a store [OFW-4.93 lv2_kernel a:0x800000000024EC6C] in a function
    whose object holds a register pointer at +136. The constructor of
    this class stores halfwords at +136
    [OFW-4.93 lv2_kernel a:0x8000000000243D8C].

## How the capture finds the objects

The kernel keeps its controllers in one 48-byte manager object, which a
getter makes on its first call [OFW-4.93 lv2_kernel a:0x800000000024FF8C].
The getter loads the TOC slot at r2 + 2944, which holds the address of
the global pointer to the manager. Its caller passes the manager to the
function at [OFW-4.93 lv2_kernel a:0x80000000002504C4], which builds
the controllers:

| Manager word | Built by | Device |
|---|---|---|
| +8 | [OFW-4.93 lv2_kernel a:0x800000000024FEA8], kind 1 | the first device argument |
| +16 | [OFW-4.93 lv2_kernel a:0x800000000024FCE0], another class | the first device argument |
| +24 | [OFW-4.93 lv2_kernel a:0x800000000024FDC4], kind 2 | the second device argument |

The builders at +8 and +24 call the constructor at
[OFW-4.93 lv2_kernel a:0x8000000000243CC8]. It stores the device object
at +40 and the kind at +52, and then the class from the TOC slot at
r2 + 2664 at +0.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD. `cellgov ps3 status` passed the profile before the reads.
- On 2026-11-20, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. The reads ran in five commands, each
  from the values that the one before it returned.
- The command read each page twice, and the two reads of every word
  agree. It wrote nothing to lv1 or lv2, used no poke, sent no system
  call or hypercall and installed nothing on the console.
- Every address is in the first 16 MiB of the kernel's address space:
  the kernel's segments and the heap after them, where earlier captures
  read the kernel's records with no fault. No row of
  `tests/micro/console_crashes.tsv` covers these addresses.
- No address is MMIO. The values at +128 and the mapping's address are
  effective addresses of the MMIO regions. The capture records them and
  reads nothing there.
- Only the 29 words of the five files are committed.
- No console-identifying value is in this directory.

## Files

| File | Rows | Holds |
|---|---|---|
| `globals.tsv` | 3 | r2, the TOC slot r2 + 2944, and the global pointer to the manager |
| `manager.tsv` | 6 | the manager's 6 doublewords |
| `controllers.tsv` | 12 | for each controller of the class: +0, +40, +48, +128, +272 and +280 |
| `devices.tsv` | 2 | +80 of each controller's device object: its MMIO mapping |
| `records.tsv` | 6 | +48, +56 and +64 of each mapping: the bus address, the effective address and the length |

Each row holds an address, the number of reads, whether they agree and
the doubleword.

## The values

| Address | Doubleword | Meaning |
|---|---|---|
| 0x8000000000003000 | 0x800000000034fbb0 | r2, as in the stock kernel |
| 0x8000000000350730 | 0x8000000000474c00 | the TOC slot: the global pointer, as in the stock kernel |
| 0x8000000000474c00 | 0x80000000004b0600 | the manager |
| 0x80000000004b0608 | 0x80000000004d4000 | the controller of kind 1 |
| 0x80000000004b0618 | 0x80000000004e5000 | the controller of kind 2 |
| 0x80000000004d4000 | 0x8000000000363760 | the class, as the stock TOC slot r2 + 2664 gives it |
| 0x80000000004d4028 | 0x80000000004ac780 | its device object |
| 0x80000000004d4030 | 0x0200000000000001 | the byte at +48 is 2; the kind at +52 is 1 |
| 0x80000000004d4080 | 0x8000000100011010 | the region's address plus 0x10 |
| 0x80000000004ac7d0 | 0x80000000004aca00 | its device's mapping |
| 0x80000000004aca30 | 0x0000000003810000 | the bus address: device 1, region 0x3810000 |
| 0x80000000004aca38 | 0x8000000100011000 | the region's effective address |
| 0x80000000004aca40 | 0x0000000000010000 | the region's length |
| 0x80000000004e5000 | 0x8000000000363760 | the class |
| 0x80000000004e5028 | 0x80000000004ce000 | its device object |
| 0x80000000004e5030 | 0x0200000000000002 | the byte at +48 is 2; the kind at +52 is 2 |
| 0x80000000004e5080 | 0x8000000100031010 | the region's address plus 0x10 |
| 0x80000000004ce050 | 0x80000000004ce180 | its device's mapping |
| 0x80000000004ce1b0 | 0x0000000003820000 | the bus address: device 2, region 0x3820000 |
| 0x80000000004ce1b8 | 0x8000000100031000 | the region's effective address |
| 0x80000000004ce1c0 | 0x0000000000010000 | the region's length |

So, for both devices:

| Register | Bits | Value |
|---|---|---|
| word +0 | the low byte | 0x10 |
| word +4 | the low 4 bits | 2 |

## Cross-checks

- Both bus addresses and both lengths are the regions that lv1 gives
  devices 1 and 2 of bus 1 (`cellgov_ps3_abi::lv1::device_bus::MMIO_REGIONS`).
- The first mapping's effective address is the address that CellGov's
  4.93 boot gives the same mapping.
- Each value at +128 less its region's address is below 0x100, as the
  sum of an address and one byte is.
- Both controllers hold the class that the constructor stores.
