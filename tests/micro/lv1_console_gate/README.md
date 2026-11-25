# lv1_console_gate

A read-only capture of lv1's logical console gate on the reference
console, at firmware 4.93. The gate decides whether lv1's logical
console calls do any work. CellGov's hypervisor reads this capture,
through `cellgov_ps3_abi::lv1::console`, to answer those calls with
the gate off.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the byte the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- webMAN's `peek.lv1` HTTP command returns 0x200 bytes of lv1 memory at
  an address. The capture used that command only, on the page at
  0x374000, after the console had booted to the XMB. It wrote nothing
  to lv1 or lv2, used no poke and installed nothing on the console.
- The page was read twice, on 2026-11-11. The two reads are identical.
- Only the gate byte is committed. The rest of the page is lv1 data
  that CellGov does not use, and the raw HTTP pages are deleted.
- No console-identifying value is in this directory.

## Where the gate is

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the address:

- The worker of hypercall 106 loads the gate's address from the TOC
  slot [OFW-4.93 lv1 a:0x359C48], which holds 0x374000, and reads the
  byte there with `lbz` [OFW-4.93 lv1 a:0x312FE8-0x313000]. The
  workers of 105, 107, 109 and 231 read the same byte.
- A gate of 2 makes the 106 worker return r3 = 0 and do nothing else
  [OFW-4.93 lv1 a:0x313070-0x313090]. The other workers do the same,
  and 107 also returns its input r4 in r4.
- The first call that finds a gate other than 1 or 2 looks up the name
  `sys.dbgcard.dgbe` [OFW-4.93 lv1 a:0x326F20]. When the lookup fails,
  the call sets the gate to 2 [OFW-4.93 lv1 a:0x313068-0x31306C], so
  the gate stays at 2 for every later call.

## Files

| File | Rows | Holds |
|---|---|---|
| `gate.tsv` | 1 | the gate's address, the number of reads, whether they agree, and the byte |

## The live gate

The gate byte is 0x02. The lookup failed on the reference console, so
lv1's logical console calls return 0 and do nothing there.
