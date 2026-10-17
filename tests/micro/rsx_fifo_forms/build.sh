#!/bin/bash
# Build the rsx_fifo_forms probe: what each RSX command header form does
# in libgcm's command buffer.
#
# Requirements -- any environment providing:
#   1. the ps3dev PPU toolchain (powerpc64-ps3-elf-gcc) and PSL1GHT,
#      installed under $PS3DEV / $PSL1GHT (default /usr/local/ps3dev;
#      override via env),
#   2. python3 (for common/patch_toc.py),
#   3. this test directory mounted at /src and the shared
#      tests/micro/common at /common (or as /src/../common).
#
#   docker run --rm -v /path/to/rsx_fifo_forms:/src \
#       -v /path/to/common:/common \
#       -e COMMON=/common ps3dev bash /src/build.sh
#
# Git Bash on Windows rewrites the /src and /common mount targets;
# prefix the command with MSYS_NO_PATHCONV=1 there.
#
set -e

PS3DEV="${PS3DEV:-/usr/local/ps3dev}"
PSL1GHT="${PSL1GHT:-$PS3DEV}"
PPU_PREFIX="powerpc64-ps3-elf"

OUT=/src/build
COMMON="${COMMON:-/src/../common}"
mkdir -p "$OUT"

${PPU_PREFIX}-gcc -c -o "$OUT/crt0.o" "$COMMON/crt0.S"
# The program imports libgcm from the firmware, so it links lv2-sprx.o
# and the stubs are fixed up with sprxlinker.
${PPU_PREFIX}-gcc -nostartfiles \
    -I${PSL1GHT}/ppu/include -I"$COMMON" -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/rsx_fifo_forms.elf" \
    "$OUT/crt0.o" \
    "$(${PPU_PREFIX}-gcc -print-file-name=lv2-sprx.o)" \
    /src/ppu/main.c \
    -lrsx -lgcm_sys -llv2 -lsysmodule -lrt
python3 "$COMMON/patch_toc.py" \
    "$OUT/rsx_fifo_forms.elf" \
    "${PPU_PREFIX}-readelf" "${PPU_PREFIX}-nm"
sprxlinker "$OUT/rsx_fifo_forms.elf"
ls -la "$OUT/rsx_fifo_forms.elf"

export CGOV_PS3_LIBS="-lrsx -lgcm_sys"
bash "$COMMON/package_ps3.sh" rsx_fifo_forms /src/ppu/main.c
