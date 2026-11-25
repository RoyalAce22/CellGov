# lv2_vuart_port_mask

A read-only capture from the 4.93 kernel's memory on the reference
console. It gives the interrupt mask that the kernel keeps for each of
the three virtual UART ports that it opens. The kernel stores the mask
at +400 of the port record, and then sends it to lv1 with
`set_virtual_uart_param` (hypercall 164), parameter 2.

lv1 can raise the port's interrupt for a mask other than 0
(`cellgov_ps3_abi::lv1::uart`). CellGov's hypervisor answers a 164 call
with such a mask only on a service port whose status shares no bit with
the mask. This capture shows that the kernel sets the mask 2 on each
port. The test `uart_set_param_console_tests` holds that to this
capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the reads returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-13, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads agree. It wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- A first peek read the TOC word, the TOC slot, the table and the count
  of the virtual UART record. A second peek read the port records that
  the table named.
- The kernel stores to each word that this capture reads, so each page
  is writable:
  - the words that the capture `lv2_vuart_port_params` reads, which its
    README lists;
  - the mask of a port record
    [OFW-4.93 lv2_kernel a:0x8000000000016ac0]
    [OFW-4.93 lv2_kernel a:0x800000000001731c]
    [OFW-4.93 lv2_kernel a:0x80000000000175c4].
- No row of `tests/micro/console_crashes.tsv` covers these words.
- No console-identifying value is in this directory.

## Where the records are

The capture `lv2_vuart_port_params` gives the virtual UART record, its
table and the port records. This capture reads the same words again,
and adds the mask of each port record.

## Where the kernel stores the mask

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these functions. Each keeps the port record
in r31, stores the new mask at +400 of the record, and sends 164 with
r4 = 2 and the new mask in r5:

- One function adds the low 3 bits of r4 to the mask
  [OFW-4.93 lv2_kernel a:0x8000000000016aa8-0x8000000000016ac8].
- One function keeps only the bits of the mask that r4 holds
  [OFW-4.93 lv2_kernel a:0x8000000000017304-0x8000000000017324].
- One function sets the mask to the low 3 bits of r4
  [OFW-4.93 lv2_kernel a:0x80000000000175b8-0x80000000000175d0].

The open function sets each trigger to the buffer size less 1, and then
the mask to 0, through the third function
[OFW-4.93 lv2_kernel a:0x80000000000177bc-0x80000000000177ec]
[OFW-4.93 lv2_kernel a:0x8000000000017644-0x8000000000017704].

## Files

| File | Rows | Holds |
|---|---|---|
| `vuart_masks.tsv` | 21 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields are:

- `toc`: the TOC word.
- `vuart`: the TOC slot of the virtual UART record.
- `slot<N>_port` and `slot<N>_record`: +72 and +80 of slot N of the
  record's table.
- `count`: the doubleword at +136 of the record. Its high word is the
  count.
- `record<N>_port`, `record<N>_tx`, `record<N>_rx` and `record<N>_mask`:
  +0, +384, +392 and +400 of the port record in slot N.

## The live records

- The TOC word, the TOC slot, the table, the count and each port record's
  port number and sizes hold the values that `lv2_vuart_port_params`
  captured. So the records are at the same addresses.
- The mask of each of the three port records is 2.
- Each function that stores the mask sends 164 with it next. So the
  kernel sent the mask 2 to each port after the open sent the mask 0.
  The capture does not show lv1's answer to that call.

| Port | Mask at +400 |
|---|---|
| 0 | 2 |
| 2 | 2 |
| 10 | 2 |
