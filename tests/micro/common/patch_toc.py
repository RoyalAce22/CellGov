"""Post-link patches for PSL1GHT ELFs running on RPCS3.

Usage: patch_toc.py <elf_path> <readelf_path> <nm_path>

Two patches:
1. TOC base: computes .got + 0x8000, writes it into __cg_tocval.
2. rldicr removal: replaces every `rldicr rN, rN, 16, 47` with
   `slwi rN, rN, 16` (a 32-bit equivalent). RPCS3 v0.0.40
   mishandles rldicr, causing branches to garbage addresses.

The rldicr scan is bounded to .text's file range -- ELF headers,
.rodata, .data, and the symbol/string tables must never be
candidates for rewriting -- and the patch count is cross-checked
against a disassembly of the same predicate. A mismatch fails the
build instead of shipping a silently corrupted ELF.
"""

import struct
import subprocess
import sys


def section_fields(readelf, elf, name):
    """(vaddr, file_offset, size) of `name` from readelf -S.

    -W keeps each section row on one line; without it, 64-bit ELF
    rows wrap and the Size column lands on the continuation line.
    """
    out = subprocess.check_output([readelf, "-S", "-W", elf], text=True)
    for line in out.splitlines():
        parts = line.split()
        if name in parts and "PROGBITS" in line:
            idx = parts.index(name)
            return (
                int(parts[idx + 2], 16),
                int(parts[idx + 3], 16),
                int(parts[idx + 4], 16),
            )
    print(f"ERROR: {name} section not found", file=sys.stderr)
    sys.exit(1)


def main():
    elf = sys.argv[1]
    readelf = sys.argv[2]
    nm = sys.argv[3]
    # readelf and objdump ship side by side in the toolchain bin dir;
    # build.sh passes only the former, so derive the latter.
    objdump = readelf.replace("readelf", "objdump")
    if objdump == readelf:
        print(f"ERROR: cannot derive objdump path from {readelf}", file=sys.stderr)
        sys.exit(1)

    got_vaddr, _, _ = section_fields(readelf, elf, ".got")
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

    text_vaddr, text_offset, text_size = section_fields(readelf, elf, ".text")

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
    expected = count_rldicr_in_disassembly(objdump, elf)
    patched = patch_rldicr(elf, text_offset, text_size)
    if patched != expected:
        print(
            f"ERROR: rldicr patch count {patched} does not match the "
            f"{expected} matching instruction(s) objdump sees in .text; "
            "the encoding predicate and the disassembler disagree",
            file=sys.stderr,
        )
        sys.exit(1)


def matches_predicate(word):
    """The exact shape patch 2 rewrites: rldicr rN, rN, 16, 47."""
    # opcode=30 (bits 0-5), sh[0:4]=16 (bits 16-20),
    # last 12 bits = 0x3E4 (me + XO + sh5 + Rc for sh=16, me=47)
    opcode = (word >> 26) & 0x3F
    sh04 = (word >> 11) & 0x1F
    tail = word & 0xFFF
    rs = (word >> 21) & 0x1F
    ra = (word >> 16) & 0x1F
    return opcode == 30 and sh04 == 16 and tail == 0x3E4 and rs == ra


def count_rldicr_in_disassembly(objdump, elf_path):
    """Count `rldicr rN,rN,16,47` sites objdump decodes in .text."""
    out = subprocess.check_output(
        [objdump, "-d", "--section=.text", elf_path], text=True
    )
    count = 0
    for line in out.splitlines():
        if "rldicr" not in line:
            continue
        operands = line.split()[-1].split(",")
        if len(operands) != 4:
            continue
        ra, rs, sh, me = operands
        if ra == rs and sh == "16" and me == "47":
            count += 1
    return count


def patch_rldicr(elf_path, text_offset, text_size):
    """Rewrite matching rldicr instructions within .text; returns the
    count patched."""
    with open(elf_path, "r+b") as f:
        data = bytearray(f.read())

    count = 0
    end = min(text_offset + text_size, len(data)) - 3
    i = text_offset
    while i < end:
        word = struct.unpack_from(">I", data, i)[0]
        if matches_predicate(word):
            rs = (word >> 21) & 0x1F
            ra = (word >> 16) & 0x1F
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
    return count


if __name__ == "__main__":
    main()
