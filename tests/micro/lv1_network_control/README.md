# lv1_network_control

A read-only capture, in lv1 on the reference console, of the two values
of the network device that lv1's `net_control` (hypercall 194) checks
before it answers a request: the device's state word and its flag byte.

CellGov's hypervisor answers the 4.93 kernel's `net_control` call at
[OFW-4.93 lv2_kernel a:0x80000000000a369c] as a device that passes both
checks. The test `network_control_console_tests` in
`cellgov_ps3_abi::lv1` holds those checks to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-17, `cellgov ps3 peek --space lv1` read the words below
  through webMAN's `peek.lv1` command, after the console had booted to
  the XMB. Each read took the address that the read before it gave. It
  read each page twice, and the two reads of every word agree. It wrote
  nothing to lv1 or lv2, used no poke, sent no hypercall and installed
  nothing on the console.
- Every address is lv1 data or lv1 heap. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No address is MMIO.
- Only the 4 words below are committed.
- No console-identifying value is in this directory. The capture did
  not read the doubleword at +16 of the device: that doubleword is the
  console's MAC address.

## The kernel's call

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps:

- The kernel sends hypercall 194 with the bus id and the device id of
  its device record in r3 and r4, the request code 1 in r5, and 0 in r6
  to r8 [OFW-4.93 lv2_kernel a:0x80000000000a3680-0x80000000000a369c].
- For an r3 other than 0 or an r4 of 0, its function returns
  0x80010005. Otherwise it keeps r4 at +56 of the record
  [OFW-4.93 lv2_kernel a:0x80000000000a36a0-0x80000000000a36d8].

When CellGov's hypervisor boots that kernel, the call has r3 = 1 and
r4 = 0: device 0 of bus 1, the network device.

## Where lv1 keeps the values

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. `cellgov_ps3_abi::lv1::network`
holds each step with its address.

- The handler loads the network device's global object from the TOC
  slot at r2 - 1848 [OFW-4.93 lv1 a:0x307DEC-0x307DF4]. The slot holds
  0x3682B0 [OFW-4.93 lv1 a:0x359900-0x359904].
- The device's object is the doubleword at +16 of the global object
  [OFW-4.93 lv1 a:0x307E28].
- The handler refuses a device whose 32-bit word at +8 is not 2
  [OFW-4.93 lv1 a:0x307E2C-0x307E40].
- The request function refuses a device whose byte at +0 is 0
  [OFW-4.93 lv1 a:0x245108-0x245118].

## Files

| File | Rows | Holds |
|---|---|---|
| `net_control.tsv` | 4 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields:

- `global`: the doubleword at 0x3682B0, the network device's global
  object.
- `device`: the doubleword at +16 of the global object, the device's
  object.
- `device_flag`: the byte at +0 of the device's object.
- `device_state`: the 32-bit word at +8 of the device's object.

## The live values

- The global object is at 0x7C110, and the device's object is at
  0x7C1A0. Both are lv1 heap addresses.
- The state word is 2. `open_device` sets it to 2 when it opens the
  device, so the game OS partition holds the device open.
- The flag byte is 1. So the request function passes the device on to
  its request switch.
- No lv1 code read stores the flag byte. CellGov's hypervisor takes it
  as 1 for an open network device, as this capture reads it.
