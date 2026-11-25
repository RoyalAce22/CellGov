# lv1_network_vlan_fields

A read-only capture, in lv1 on the reference console at firmware 4.93,
of the two fields of the network device's global object that
`net_control` (hypercall 194) reads for request 4, the VLAN id request:
- the doubleword at +32. When its bit 63 is set, request 4 gives -6 for
  each selector;
- the byte at +28. Selector 5 gives the id 294 when it is 0, and 296
  otherwise.

`cellgov_ps3_abi`'s `network_control_vlan_fields_console_tests` holds
`CONSOLE_VLAN_FIELDS` to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words that the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-17, after the console had booted to the XMB,
  `cellgov ps3 peek --space lv1` read the three words below through
  webMAN's `peek.lv1` command, one command for each word:

  ```text
  cellgov ps3 peek --space lv1 --width 8 --address 0x3682B0
  cellgov ps3 peek --space lv1 --width 1 --address 0x7C12C
  cellgov ps3 peek --space lv1 --width 8 --address 0x7C130
  ```

- Each command read its page twice, and the two reads of each word
  agree. The runner wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- Every address is lv1 data or lv1 heap. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No address is MMIO.
- Only the 3 words below are committed. The page of lv1 heap that holds
  the two fields also holds the console's MAC address. The runner writes
  only the words that a command names, so no other word of that page is
  in this directory.

## Where lv1 reads the fields

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. `cellgov_ps3_abi::lv1::network`
holds each step with its address.

- Request 4's case loads the global object through the TOC slot at
  r2 - 24896, which holds 0x3682B0 [OFW-4.93 lv1 a:0x244DD8-0x244DDC]
  [OFW-4.93 lv1 a:0x353EF8-0x353EFC].
- It loads the doubleword at +32 of the global object and goes to the
  failure -6 when bit 63 is set [OFW-4.93 lv1 a:0x244DE0-0x244DEC]
  [OFW-4.93 lv1 a:0x245014-0x245018]. This test comes before the
  selector's range check.
- Selector 5's case loads the byte at +28 of the global object, and
  gives 294 when it is 0 and 296 otherwise
  [OFW-4.93 lv1 a:0x244EC8-0x244EE0] [OFW-4.93 lv1 a:0x244E88-0x244E8C].

No lv1 code read stores either field. The code that stores the byte at
+27 of the global object is at [OFW-4.93 lv1 a:0x23E22C] and
[OFW-4.93 lv1 a:0x23E634]. No store to +28 or +32 of the global object
is found through any TOC slot that holds 0x3682B0.

## Files

| File | Rows | Holds |
|---|---|---|
| `fields.tsv` | 3 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields:

- `global`: the doubleword at 0x3682B0, the address of the network
  device's global object.
- `vlan_flag`: the byte at +28 of the global object.
- `vlan_capability`: the doubleword at +32 of the global object.

## The live values

- The global object is at 0x7C110, as the capture
  `lv1_network_control` also reads it.
- The doubleword at +32 is 1, so bit 63 is set. Request 4 gives -6 for
  each selector, and the selector's range check and table do not run.
- The byte at +28 is 0. Selector 5 would give 294 if bit 63 were clear.

## What the capture does not settle

The capture reads the fields after the boot to the XMB. It does not
show the values at the time of the kernel's first request 4. CellGov's
hypervisor takes the fields as this capture reads them.
