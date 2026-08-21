#!/bin/bash
# Build the process_spawn_wait microtest (parent + child ELFs).
#
# Requirements -- any environment providing:
#   1. the ps3dev PPU toolchain (powerpc64-ps3-elf-gcc) and
#      PSL1GHT, installed under $PS3DEV / $PSL1GHT (default
#      /usr/local/ps3dev, the ps3toolchain standard prefix;
#      override via env),
#   2. python3 (for common/patch_toc.py),
#   3. this test directory mounted/available at /src and the shared
#      tests/micro/common at /common (or as /src/../common).
#
# Any ps3dev+PSL1GHT container works, e.g. one built from the
# ps3dev/ps3toolchain and ps3dev/PSL1GHT projects:
#
#   docker run --rm -v /path/to/process_spawn_wait:/src \
#       -v /path/to/common:/common \
#       -e COMMON=/common <your-ps3dev-psl1ght-image> bash /src/build.sh
#
# Git Bash on Windows rewrites the /src and /common mount targets
# to Windows paths, which leaves stray "<dir>;C" directories on the
# host; prefix the command with MSYS_NO_PATHCONV=1 there.
#
set -e

PS3DEV="${PS3DEV:-/usr/local/ps3dev}"
PSL1GHT="${PSL1GHT:-$PS3DEV}"
PPU_PREFIX="powerpc64-ps3-elf"

OUT=/src/build
COMMON="${COMMON:-/src/../common}"
mkdir -p "$OUT"

echo "=== Assembling custom CRT0 ==="
${PPU_PREFIX}-gcc -c -o "$OUT/crt0.o" "$COMMON/crt0.S"

for prog in parent child; do
  echo "=== Linking $prog ==="
  ${PPU_PREFIX}-gcc \
      -nostartfiles \
      -I${PSL1GHT}/ppu/include \
      -L${PSL1GHT}/ppu/lib \
      -O2 -Wall \
      -o "$OUT/$prog.elf" \
      "$OUT/crt0.o" \
      /src/ppu/$prog.c \
      -llv2 -lsysmodule -lrt

  echo "=== Patching TOC and rldicr ($prog) ==="
  python3 "$COMMON/patch_toc.py" \
      "$OUT/$prog.elf" \
      "${PPU_PREFIX}-readelf" \
      "${PPU_PREFIX}-nm"
done

echo "=== Wrapping child.elf as a genuinely SCE-wrapped SELF ==="
# make_self produces an APP-keyed encrypted SELF -- the shape a real
# spawned child arrives in -- so the spawn loader's SCE unwrap path
# is exercised by a real container, not a raw ELF renamed .self.
# (fself's fake-SELF layout has no CTR metadata directory and is not
# in the decrypt path's scope.)
make_self "$OUT/child.elf" "$OUT/child.self"

echo "=== Build complete ==="
ls -la "$OUT"/parent.elf "$OUT"/child.elf "$OUT"/child.self
