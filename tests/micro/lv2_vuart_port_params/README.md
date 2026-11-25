# lv2_vuart_port_params

A read-only capture from the 4.93 kernel's memory on the reference
console. It gives the buffer sizes that lv1's `get_virtual_uart_param`
(hypercall 165) gave the kernel's calls for the three virtual UART
ports that the kernel opens. The kernel keeps the transmit buffer size
at +384 and the receive buffer size at +392 of each port record.

CellGov's hypervisor answers those calls with these sizes through
`GAME_OS_VIRTUAL_UART_PORTS` in `cellgov_ps3_abi::lv1::uart`. The test
`uart_param_console_tests` holds the rows to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the reads returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-12, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads agree. It wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- The kernel stores to each word that this capture reads, so each page
  is writable:
  - the table and the count of the virtual UART record
    [OFW-4.93 lv2_kernel a:0x800000000001796c-0x8000000000017978];
  - the port number of a port record
    [OFW-4.93 lv2_kernel a:0x80000000000169d0];
  - the two sizes of a port record
    [OFW-4.93 lv2_kernel a:0x8000000000017740]
    [OFW-4.93 lv2_kernel a:0x800000000001775c].
- No row of `tests/micro/console_crashes.tsv` covers these words.
- No console-identifying value is in this directory.

## Where the records are

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses. The kernel's r2 is
0x800000000034fbb0 [OFW-4.93 lv2_kernel a:0x8000000000003000].

- The TOC slot r2 - 30880 holds the virtual UART record
  0x800000000037dc40 [OFW-4.93 lv2_kernel a:0x8000000000348310]. The
  capture `lv2_vuart_irq_outlet` reads the same record through the slot
  r2 - 30904.
- The record's init function sets the four port numbers at +72, +88,
  +104 and +120 to 256, and the count at +136 to 0
  [OFW-4.93 lv2_kernel a:0x8000000000017c48-0x8000000000017c70].
- The kernel's register function adds a port to the table only when the
  count is 3 or less
  [OFW-4.93 lv2_kernel a:0x8000000000017930-0x8000000000017940]. It
  stores the port number at +72 and the port record at +80 of the slot
  that the count names, 16 bytes each, and adds 1 to the count
  [OFW-4.93 lv2_kernel a:0x8000000000017958-0x8000000000017978].
- The kernel opens a port with the function at 0x8000000000017708, and
  registers the port only when that function returns 0. The open paths
  for ports 0 and 2 call the record's init function with the port
  number in r4
  [OFW-4.93 lv2_kernel a:0x80000000000183c0-0x80000000000183e8]
  [OFW-4.93 lv2_kernel a:0x8000000000264388-0x800000000026439c]. The
  init function stores r4 at +0 of the record
  [OFW-4.93 lv2_kernel a:0x80000000000169d0].

## Where the kernel stores the sizes

- The open function keeps r3, the record, in r31. It loads the port
  number from +0 and sends hypercall 165 with r4 = 3
  [OFW-4.93 lv2_kernel a:0x8000000000017714-0x8000000000017730].
- On r3 = 0 it stores r4 at +392 of the record. It then sends hypercall
  165 with r4 = 5
  [OFW-4.93 lv2_kernel a:0x8000000000017734-0x8000000000017750].
- On r3 = 0 it stores r4 at +384 of the record
  [OFW-4.93 lv2_kernel a:0x8000000000017754-0x800000000001775c].
- On r3 = 0 for both calls it gives each size less 1 to its trigger
  step, which sends `set_virtual_uart_param` (hypercall 164)
  [OFW-4.93 lv2_kernel a:0x80000000000177bc-0x80000000000177ec].

## Files

| File | Rows | Holds |
|---|---|---|
| `vuart_ports.tsv` | 20 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields are:

- `toc`: the TOC word.
- `vuart`: the TOC slot of the virtual UART record.
- `slot<N>_port` and `slot<N>_record`: +72 and +80 of slot N of the
  record's table.
- `count`: the doubleword at +136 of the record. Its high word is the
  count.
- `record<N>_port`, `record<N>_tx` and `record<N>_rx`: +0, +384 and +392
  of the port record in slot N.

## The live records

- The TOC word and the TOC slot hold their stock values, so the record
  is at the address that the image gives.
- The count is 3. Slots 0 to 2 hold ports 10, 2 and 0, and slot 3 holds
  256, the init value.
- The port number at +0 of each record is the port number of its slot.
- The open function returns 0x80010016 when either call returns
  `LV1_NO_ENTRY`, and stops the kernel on another failure
  [OFW-4.93 lv2_kernel a:0x8000000000017760-0x8000000000017798]. The
  kernel registered each port, so both calls returned r3 = 0 for each
  port, and each record holds lv1's answers.

| Port | Transmit buffer | Receive buffer |
|---|---|---|
| 0 | 0x800 | 0x800 |
| 2 | 0x800 | 0x800 |
| 10 | 0x1000 | 0x1000 |
