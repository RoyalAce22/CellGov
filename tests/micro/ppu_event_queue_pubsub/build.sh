#!/bin/bash
set -e

PS3DEV="${PS3DEV:-/usr/local/ps3dev}"
PSL1GHT="${PSL1GHT:-$PS3DEV}"
PPU_PREFIX="powerpc64-ps3-elf"

OUT=/src/build
COMMON="${COMMON:-/src/../common}"
mkdir -p "$OUT"

${PPU_PREFIX}-gcc -c -o "$OUT/crt0.o" "$COMMON/crt0.S"
${PPU_PREFIX}-gcc -nostartfiles \
    -I${PSL1GHT}/ppu/include -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/ppu_event_queue_pubsub.elf" \
    "$OUT/crt0.o" /src/ppu/main.c \
    -llv2 -lsysmodule -lrt
python3 "$COMMON/patch_toc.py" \
    "$OUT/ppu_event_queue_pubsub.elf" \
    "${PPU_PREFIX}-readelf" "${PPU_PREFIX}-nm"
ls -la "$OUT/ppu_event_queue_pubsub.elf"
