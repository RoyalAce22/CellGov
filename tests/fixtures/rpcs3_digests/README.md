# RPCS3 reference digests

`digests.txt` pins what RPCS3 produces for inputs CellGov also
processes. The parity tests hash CellGov's output and compare against
these, so the expected value is committed data rather than a live RPCS3
install tree that only one machine has.

## Format

```
<sha256>  <key>  <bytes>
```

Lines starting with `#` are comments. Keys:

| Key | What RPCS3 produced |
| --- | --- |
| `eboot/<content-id>` | `EBOOT.BIN` as RPCS3 extracted it from the title's PKG or disc image -- still SCE-wrapped, not decrypted |
| `decrypted_masked/<module>` | `<module>.prx`, RPCS3's plaintext of the firmware `<module>.sprx`, after `mask_non_semantic_elf_bytes` zeroes `e_shoff` / `e_shnum` / `e_shstrndx` -- the same mask the parity test applies to CellGov's output before comparing |

## What a mismatch means

A failing parity test means CellGov's installer or decrypt pipeline
stopped agreeing with RPCS3 for that input. Investigate the
divergence; do NOT re-bless to make the test pass.

Re-blessing is correct only when the reference itself legitimately
changed -- a different firmware revision, a different dump of the
title, or a corrected RPCS3 extraction.

## Re-blessing

The digests come from an RPCS3 install tree, which is an operator
artifact and is not committed. With one present at `tools/rpcs3/`,
run this from the workspace root -- the paths below are relative to
it:

```bash
python - <<'PY'
import hashlib, os
MODULES = ["libaudio","libfs","libgcm_sys","libio","liblv2","libnet",
           "libnetctl","libspurs_jq","libsync2","libsysmodule",
           "libsysutil","libsysutil_np"]

def mask(b):
    # Mirrors cellgov_install::sce::mask_non_semantic_elf_bytes:
    # zero e_shoff, e_shnum, e_shstrndx. NUL bytes, not spaces.
    if len(b) < 0x40: return b
    b = bytearray(b)
    b[0x28:0x30] = bytes(8)
    b[0x3C:0x3E] = bytes(2)
    b[0x3E:0x40] = bytes(2)
    return bytes(b)

srcs = [
    ("eboot/NPUA80001", "tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN"),
    ("eboot/NPUA80068", "tools/rpcs3/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.BIN"),
    ("eboot/BCES00664", "tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.BIN"),
] + [("decrypted_masked/" + m,
      "tools/rpcs3/dev_flash_decrypted/sys/external/%s.prx" % m) for m in MODULES]
for key, path in srcs:
    if not os.path.isfile(path):
        print("ABSENT", path); continue
    data = open(path, "rb").read()
    if key.startswith("decrypted_masked/"):
        data = mask(data)
    print("%s  %s  %d" % (hashlib.sha256(data).hexdigest(), key, len(data)))
PY
```

Replace the rows under the header in `digests.txt` with the output --
do not append. A key that appears twice is rejected when the table is
read, rather than resolved by which row came last. Say in the commit
message which reference changed and why.

The recipe reproduces the committed rows exactly when the inputs are
unchanged, so diffing its output against the data rows of
`digests.txt` -- it does not reprint the leading comment block -- is
the check that the tree you re-blessed from is the one already
pinned. `ABSENT` on a line means that input is missing from the tree;
those keys keep their committed rows.
