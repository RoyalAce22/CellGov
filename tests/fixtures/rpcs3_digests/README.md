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
| `sprx/<version>/<module>` | `<module>.sprx` as RPCS3's install of firmware `<version>` wrote it -- still SCE-wrapped. The input the `decrypted_masked/<module>` row was decrypted from. Every such row names one version, and the parity test decrypts that version's install |

## What a mismatch means

A failing parity test means CellGov's installer or decrypt pipeline
stopped agreeing with RPCS3 for that input. Investigate the
divergence; do NOT re-bless to make the test pass.

The firmware suite holds each installed `<module>.sprx` against its
`sprx/` row before it decrypts. When that input check fails, the
reference plaintext was captured from a different file, and the
decrypt was never compared. When the input matches and the plaintext
does not, the decrypt is at fault.

Re-blessing is correct only when the reference itself legitimately
changed -- a different firmware revision, a different dump of the
title, or a corrected RPCS3 extraction.

## Re-blessing

The digests come from an RPCS3 install tree, which is an operator
artifact and is not committed. With one present at `tools/rpcs3/` --
the reference firmware installed into its `dev_flash/`, the three
titles into `dev_hdd0/` and `dev_bdvd/` -- run this from the
workspace root. The paths below are relative to it:

```bash
MODULES="libaudio libfs libgcm_sys libio liblv2 libnet libnetctl
         libspurs_jq libsync2 libsysmodule libsysutil libsysutil_np"
WORK=$(mktemp -d)
args=()
for m in $MODULES; do
  cp "tools/rpcs3/dev_flash/sys/external/$m.sprx" "$WORK/"
  args+=(--decrypt "$WORK/$m.sprx")
done
# RPCS3 writes <module>.prx beside each copy. It holds RPCS3.buf while
# it runs and refuses to start while one exists; a --decrypt run exits
# without removing it, so the run removes the lock it made, and only
# that one.
if [ -e tools/rpcs3/RPCS3.buf ]; then
  echo "tools/rpcs3/RPCS3.buf exists: an RPCS3 is running, or one left it behind" >&2
elif tools/rpcs3/rpcs3.exe --headless "${args[@]}" </dev/null >&2; then
  rm -f tools/rpcs3/RPCS3.buf
fi

python - "$WORK" $MODULES <<'PY'
import hashlib, os, sys
work, modules = sys.argv[1], sys.argv[2:]
flash = "tools/rpcs3/dev_flash"

def mask(b):
    # Mirrors cellgov_install::sce::mask_non_semantic_elf_bytes:
    # zero e_shoff, e_shnum, e_shstrndx. NUL bytes, not spaces.
    if len(b) < 0x40: return b
    b = bytearray(b)
    b[0x28:0x30] = bytes(8)
    b[0x3C:0x3E] = bytes(2)
    b[0x3E:0x40] = bytes(2)
    return bytes(b)

def row(key, path, masked=False):
    if not os.path.isfile(path):
        print("ABSENT", path); return
    data = open(path, "rb").read()
    if masked:
        data = mask(data)
    print("%s  %s  %d" % (hashlib.sha256(data).hexdigest(), key, len(data)))

# "release:04.9300:" names 4.93, the version the store keys the entry on.
release = next(l for l in open(flash + "/vsh/etc/version.txt")
               if l.startswith("release:"))
major, minor = release.split(":")[1].split(".")
fw = "%d.%s" % (int(major), minor[:2])

row("eboot/NPUA80001", "tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN")
row("eboot/NPUA80068", "tools/rpcs3/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.BIN")
row("eboot/BCES00664", "tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.BIN")
for m in modules:
    row("decrypted_masked/" + m, os.path.join(work, m + ".prx"), masked=True)
for m in modules:
    row("sprx/%s/%s" % (fw, m), "%s/sys/external/%s.sprx" % (flash, m))
PY
```

Every plaintext row and the input row beside it come from one
`dev_flash/`, so no row can pair one revision's plaintext with
another's input.

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
