# lv1_network_vlan_id

A read-only capture, in lv1 on the reference console at firmware 4.93,
of the code and the data that `net_control` (hypercall 194) runs for
request 4, the VLAN id request. It gives these values:
- the TOC slot of the network device's global object and the TOC slot
  of the request's jump table;
- for each selector from 0 to 20, the table's entry, the case that the
  entry leads to, and the id that the case gives;
- the immediates of the range check, of the two failure statuses and of
  the two offsets in the global object that the case reads;
- for each code range of the request's path that
  `cellgov_ps3_abi::lv1::network` cites, whether the live words are the
  words of the shipped image.

`cellgov_ps3_abi`'s `network_control_vlan_console_tests` holds
`net_control_vlan_selector`, `NET_CONTROL_VLAN_SELECTOR_MAX` and the two
failure statuses to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the values that the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-17, `runner_ps3 peek --space lv1 --width 8` read 65
  doublewords through webMAN's `peek.lv1` command, after the console had
  booted to the XMB. It read each page twice, and the two reads of each
  doubleword agree.
- It wrote nothing to lv1 or lv2, used no poke, sent no hypercall and
  installed nothing on the console.
- Every address is lv1 code or an lv1 TOC slot. No row of
  `tests/micro/console_crashes.tsv` covers these addresses. No address
  is MMIO.
- The code words are not committed. `code.tsv` records only whether
  they equal the shipped image. `table.tsv` and `immediates.tsv` record
  values that the live words give.
- No console-identifying value is in this directory. The capture did
  not read the global object or the device's object: the page of lv1
  heap that holds them also holds the console's MAC address.

## Where the words are

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses. `cellgov_ps3_abi::lv1::network`
holds each step with its address. The TOC base of the request's
functions is 0x35A038, so the slot at r2 - 24896 is at 0x353EF8 and the
slot at r2 - 24768 is at 0x353F78.

- The switch sends request 4 to its case at 0x244DD8
  [OFW-4.93 lv1 a:0x244ADC-0x244AE0] [OFW-4.93 lv1 a:0x244B58-0x244B74].
- The case loads the global object through the slot at r2 - 24896
  [OFW-4.93 lv1 a:0x244DD8-0x244DDC], and tests bit 63 of the
  doubleword at +32 of the global object
  [OFW-4.93 lv1 a:0x244DE0-0x244DEC].
- The case fails a selector above the range check's immediate
  [OFW-4.93 lv1 a:0x244DF0-0x244DF4], and branches through the table in
  the slot at r2 - 24768 [OFW-4.93 lv1 a:0x244DF8-0x244E10].
- An entry of the table is a signed offset from the table's address
  [OFW-4.93 lv1 a:0x244E00-0x244E08].

## Files

| File | Rows | Holds |
|---|---|---|
| `words.tsv` | 2 | for each TOC slot: its address, its role, the number of reads, whether they agree, and its value |
| `table.tsv` | 21 | for each selector: the entry's address, the entry, the case's address, the case's kind, and the case's id |
| `immediates.tsv` | 5 | for each instruction: its address, the field that its immediate gives, and the immediate, signed |
| `code.tsv` | 7 | for each code range: its first and last byte, what it holds, the doublewords read, whether each doubleword's two reads agree, and how it compares with the stock image |

The kinds in `table.tsv`:
- `illegal`: the case is the instruction at 0x244B24, which loads -17
  as the status [OFW-4.93 lv1 a:0x244B24];
- `fixed`: the case's first instruction is `li r11` with the id as its
  immediate. `id` is that immediate;
- `flagged`: any other case. Only selector 5 has one. Its first
  instruction loads the byte at +28 of the global object
  [OFW-4.93 lv1 a:0x244EC8-0x244EE0].

The fields in `immediates.tsv`:
- `selector_max`: the immediate of the range check's `cmpldi`;
- `illegal_status`: the status that 0x244B24 loads;
- `no_entry_status`: the status that 0x245014 loads, when bit 63 of the
  doubleword at +32 is set;
- `capability_offset`: the offset of that doubleword in the global
  object;
- `flag_offset`: the offset in the global object of the byte that
  selector 5 tests.

## The live values

- The slot at r2 - 24896 holds 0x3682B0, the address that the slot at
  r2 - 1848 holds (see the capture `lv1_network_control`). The slot at
  r2 - 24768 holds the table's address, 0x244E14.
- Selectors 1 to 4 give 292, 294, 295 and 291. Selectors 16 to 20 give
  288, 289, 290, 261 and 256. Selectors 0 and 6 to 15 lead to the
  failure at 0x244B24. Selector 5 tests the byte at +28.
- The range check passes selectors up to 20. The failures load -17 and
  -6.
- Each of the 63 code doublewords is the doubleword of the shipped
  image.

## What the capture does not settle

The capture shows the code and the table that the console runs. It does
not run the call. A user process cannot make it: a problem-state `sc 1`
to a number below 256 gets r3 = 0x00000000FFFFFFFD from the vector (the
captures `lv1_problem_state_sc` and `lv1_hypercall_table`).

The capture does not read bit 63 of the doubleword at +32 or the byte
at +28 of the global object. The capture `lv1_network_vlan_fields` reads
both: the doubleword at +32 is 1, so bit 63 is set, and lv1 gives -6
for each selector before the table is read. CellGov answers from those
fields (`CONSOLE_VLAN_FIELDS`).
