"""Rewrite a SELF's control_flags1, the first body word of its plaintext
capability record (supplemental header type 1).

    patch_ctrl_flags.py <self> <value>

make_self_npdrm writes 0x40000000, the root bit. A retail title carries
0. The record sits in the plaintext header, so the rewrite needs no key;
whether the console accepts the changed header is the console's answer.
Exits non-zero, leaving the file untouched, when the SELF carries no
type-1 record.
"""
import struct
import sys

path, value = sys.argv[1], int(sys.argv[2], 0)
with open(path, "rb") as fh:
    data = bytearray(fh.read())
if data[:4] != b"SCE\0":
    sys.exit("%s: not an SCE container" % path)
supp_off, supp_size = struct.unpack(">QQ", data[0x58:0x68])
off, end = supp_off, supp_off + supp_size
while off + 16 <= end:
    kind, size = struct.unpack(">II", data[off:off + 8])
    if kind == 1:
        old = struct.unpack(">I", data[off + 16:off + 20])[0]
        data[off + 16:off + 20] = struct.pack(">I", value)
        with open(path, "wb") as fh:
            fh.write(data)
        print("control_flags1 at 0x%x: 0x%08x -> 0x%08x" % (off + 16, old, value))
        sys.exit(0)
    if size == 0:
        break
    off += size
sys.exit("%s: no capability record" % path)
