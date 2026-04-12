"""Post-link patches for PSL1GHT ELFs running on RPCS3.

Usage: patch_toc.py <elf_path> <readelf_path> <nm_path>

Two patches:
1. TOC base: computes .got + 0x8000, writes it into __cg_tocval.
2. rldicr removal: replaces every `rldicr rN, rN, 16, 47` with
   `slwi rN, rN, 16` (a 32-bit equivalent). RPCS3 v0.0.40
   mishandles rldicr, causing branches to garbage addresses.
"""

import struct
import subprocess
import sys


def main():
    elf = sys.argv[1]
    readelf = sys.argv[2]
    nm = sys.argv[3]

    # Extract .got vaddr from section headers
    out = subprocess.check_output([readelf, "-S", elf], text=True)
    got_vaddr = None
    for line in out.splitlines():
        parts = line.split()
        if ".got" in parts and "PROGBITS" in line:
            # Format: [N] .got PROGBITS <addr> <offset> <size> ...
            idx = parts.index(".got")
            got_vaddr = int(parts[idx + 2], 16)
            break
    if got_vaddr is None:
        print("ERROR: .got section not found", file=sys.stderr)
        sys.exit(1)

    toc = got_vaddr + 0x8000
    print(f"GOT=0x{got_vaddr:x} TOC=0x{toc:08x}")

    # Find __cg_tocval vaddr
    out = subprocess.check_output([nm, elf], text=True)
    tv_vaddr = None
    for line in out.splitlines():
        if "__cg_tocval" in line:
            tv_vaddr = int(line.split()[0], 16)
            break
    if tv_vaddr is None:
        print("ERROR: __cg_tocval symbol not found", file=sys.stderr)
        sys.exit(1)

    # Find .text section vaddr and file offset for offset computation
    out = subprocess.check_output([readelf, "-S", elf], text=True)
    text_vaddr = None
    text_offset = None
    for line in out.splitlines():
        parts = line.split()
        if ".text" in parts and "PROGBITS" in line:
            idx = parts.index(".text")
            text_vaddr = int(parts[idx + 2], 16)
            text_offset = int(parts[idx + 3], 16)
            break
    if text_vaddr is None:
        print("ERROR: .text section not found", file=sys.stderr)
        sys.exit(1)

    file_offset = tv_vaddr - text_vaddr + text_offset
    print(f"Patching 0x{toc:08x} at file offset 0x{file_offset:x}")

    with open(elf, "r+b") as f:
        f.seek(file_offset)
        f.write(struct.pack(">I", toc))

    print("TOC patched")

    # Patch 2: replace rldicr with 32-bit equivalents.
    # rldicr rN, rN, 16, 47 = rotate left doubleword then clear right.
    # Encoding: 30 | rS<<21 | rA<<16 | sh[0:4]<<11 | mb[5]|mb[0:4]<<5 | XO=1 | sh[5] | Rc=0
    # For sh=16, mb=47: specific bit patterns.
    # We replace with rlwinm rA, rS, 16, 0, 15 (slwi rN, rN, 16).
    patch_rldicr(elf)


def patch_rldicr(elf_path):
    """Replace rldicr instructions with 32-bit equivalents."""
    with open(elf_path, "r+b") as f:
        data = bytearray(f.read())

    count = 0
    # Scan for rldicr rN, rN, 16, 47 pattern.
    # rldicr encoding (MD-form): opcode 30, SH=16, ME=47
    # Bits: 011110 SSSSS AAAAA sssss MMMMM 01 S 0
    # sh=16: sh[0:4]=10000=16, sh[5]=0
    # me=47: me[5]=1, me[0:4]=01111 -> me_field = 1|01111<<1 = 0b111110 = stored as me[5]||me[0:3] in bits 21-25
    # Actually ME is encoded differently. Let me just match the known byte pattern.
    # From the disassembly: 79 29 83 e4 = rldicr r9,r9,16,47
    # Let me decode: 0x7929_83E4
    # Bits: 0111 1001 0010 1001 1000 0011 1110 0100
    #   opcode = 011110 = 30 (rldicr)
    #   rS = 01001 = 9
    #   rA = 01001 = 9
    #   sh[0:4] = 10000 = 16
    #   me[5]||me[0:3] = 01111 -> hmm
    # Encoding bits 21-25: me field = 01111, bit 26-28: XO=1 (01), bit 29: sh[5]=1, bit 30: Rc=0
    # Wait, bits 26-29 = 0011 1110 0100 -> let me reparse
    # 0x7929_83E4 in binary:
    # 7: 0111  9: 1001  2: 0010  9: 1001  8: 1000  3: 0011  E: 1110  4: 0100
    # 01111001 00101001 10000011 11100100
    # opcode[0:5] = 011110 = 30 OK
    # rS[6:10] = 01001 = 9
    # rA[11:15] = 01001 = 9
    # sh[16:20] = 10000 = 16
    # mb[21:25] = 01111
    # XO[26:28] = 100 -> XO=1 for rldicr is bits 27-29 = 01, not 100
    # Hmm, let me re-check the PPC encoding.

    # Rather than decode the full encoding, just search for the known
    # 4-byte patterns. Each rldicr rN,rN,16,47 has a fixed encoding
    # except for the register field.
    #
    # rldicr encoding for sh=16, me=47:
    # The last 16 bits are always 0x83E4 for these parameters.
    # The first 16 bits encode: opcode(6) | rS(5) | rA(5).
    # opcode = 30 = 0b011110, so first byte starts with 0x78 or 0x79.

    i = 0
    while i < len(data) - 3:
        word = struct.unpack_from(">I", data, i)[0]
        # Check: opcode=30 (bits 0-5), sh[0:4]=16 (bits 16-20),
        # last 12 bits = 0x3E4 (me + XO + sh5 + Rc for sh=16, me=47)
        opcode = (word >> 26) & 0x3F
        sh04 = (word >> 11) & 0x1F
        tail = word & 0xFFF
        rs = (word >> 21) & 0x1F
        ra = (word >> 16) & 0x1F

        if opcode == 30 and sh04 == 16 and tail == 0x3E4 and rs == ra:
            # Replace with: rlwinm rA, rS, 16, 0, 15  (= slwi rN, rN, 16)
            # rlwinm encoding: opcode=21, rS, rA, SH=16, MB=0, ME=15, Rc=0
            # 21<<26 | rS<<21 | rA<<16 | 16<<11 | 0<<6 | 15<<1 | 0
            replacement = (21 << 26) | (rs << 21) | (ra << 16) | (16 << 11) | (0 << 6) | (15 << 1)
            struct.pack_into(">I", data, i, replacement)
            count += 1

        i += 4

    if count > 0:
        with open(elf_path, "wb") as f:
            f.write(data)
        print(f"Patched {count} rldicr instructions")
    else:
        print("No rldicr instructions found")


if __name__ == "__main__":
    main()
