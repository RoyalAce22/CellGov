# lv2_vuart_irq_outlet

A read-only capture from the 4.93 kernel's memory on the reference
console. It gives the irq outlet id that lv1's
`configure_virtual_uart_irq` (hypercall 166) gave the kernel's call at
[OFW-4.93 lv2_kernel a:0x8000000000017b0c]. The kernel keeps that id at
+0x08 of its virtual UART interrupt record.

CellGov's hypervisor answers that call with this id through
`GAME_OS_VIRTUAL_UART_IRQ_CALLS` in `cellgov_ps3_abi::lv1::uart`. The
test `uart_console_tests` holds the row to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-12, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads agree. It wrote nothing to lv1 or lv2, used no poke and
  installed nothing on the console.
- The kernel stores to the record
  [OFW-4.93 lv2_kernel a:0x8000000000017b20], so its page is writable.
  The TOC slot is in the same image as the TOC slot that
  `lv2_record_port_ids` reads. No row of `tests/micro/console_crashes.tsv`
  covers these words.
- Only the three words below are committed.
- No console-identifying value is in this directory.

## Where the record is

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses. The kernel's r2 is
0x800000000034fbb0 [OFW-4.93 lv2_kernel a:0x8000000000003000].

- The kernel's init function at 0x8000000000017c8c loads the TOC slot
  r2 - 30904 [OFW-4.93 lv2_kernel a:0x8000000000017c9c]. The image holds
  0x800000000037dc40 in that slot [OFW-4.93 lv2_kernel a:0x80000000003482f8].
- The init function gives that address to the function at
  0x8000000000017ad0 in r3, and calls it again until it returns 0
  [OFW-4.93 lv2_kernel a:0x8000000000017cac-0x8000000000017cb8].

## Where the kernel stores the id

- The function keeps r3, the record, in r29
  [OFW-4.93 lv2_kernel a:0x8000000000017ae4].
- It gives the address of +0x28 of the record and a stack slot to the
  function at 0x8000000000077760. It loads that slot into r3, and sends
  hypercall 166
  [OFW-4.93 lv2_kernel a:0x8000000000017ae0-0x8000000000017b0c].
- On r3 = 0 it stores r4 at +0x08 of the record
  [OFW-4.93 lv2_kernel a:0x8000000000017b10-0x8000000000017b20]. It then
  gives that id in r3 to the kernel's connect step
  [OFW-4.93 lv2_kernel a:0x8000000000017b2c-0x8000000000017b38], which
  sends `connect_irq_plug_ext` (hypercall 12).

## Files

| File | Rows | Holds |
|---|---|---|
| `vuart_outlet.tsv` | 3 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields are `toc` (the TOC word), `vuart` (the TOC slot of the
record) and `outlet` (+0x08 of the record).

## The live record

- The TOC word and the TOC slot hold their stock values, so the record
  is at the address that the image gives.
- The outlet id is 18 (0x12). It is not 0, so the call returned r3 = 0.
- lv1 gives the outlet the first free id of the partition's object
  table; see `cellgov_ps3_abi::lv1::uart`. The captures
  `lv2_event_port_ids`, `lv2_record_port_ids` and
  `lv2_logical_spe_object` give ids 3 to 17 to the kernel's event
  receive ports, its boot logical SPE and that SPE's outlets. The logical
  PPE has id 1 and the boot address space id 2. So 18 is the first id
  above them, and fits a call after the last of those objects.
