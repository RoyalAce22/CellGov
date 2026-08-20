#!/bin/bash
# Build the rsx_flip_status_transition microtest.
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
#   docker run --rm -v /path/to/rsx_flip_status_transition:/src \
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

echo "=== Linking PPU program ==="
${PPU_PREFIX}-gcc \
    -nostartfiles \
    -I${PSL1GHT}/ppu/include \
    -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/rsx_flip_status_transition.elf" \
    "$OUT/crt0.o" \
    /src/ppu/main.c \
    -llv2 -lsysmodule -lrt

echo "=== Patching TOC and rldicr ==="
python3 "$COMMON/patch_toc.py" \
    "$OUT/rsx_flip_status_transition.elf" \
    "${PPU_PREFIX}-readelf" \
    "${PPU_PREFIX}-nm"

echo "=== Build complete ==="
ls -la "$OUT/rsx_flip_status_transition.elf"
