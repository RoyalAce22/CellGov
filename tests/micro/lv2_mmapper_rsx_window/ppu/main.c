/* PPU program: where sys_mmapper_allocate_address (330) places a game
 * process's 256 MiB areas, before and after libgcm opens an RSX
 * context.
 *
 * Part 1 reserves up to eight areas, recording each call's r3 and the
 * base it wrote, then frees every area it got. Part 2 brings up an RSX
 * context with rsxInit and records libgcm's control register. Part 3
 * repeats part 1. A failed reserve stops its part.
 *
 * Payload (34 big-endian u64):
 *   0       rsxInit rc (0xFFFFFFFFFFFFFFFF until part 2 runs)
 *   1       libgcm's control register address (0 when rsxInit fails)
 *   2..9    part 1 bases (0 for a reserve that failed or was not sent)
 *   10..17  part 1 r3 per reserve (SKIP when not sent)
 *   18..25  part 3 bases
 *   26..33  part 3 r3 per reserve
 *
 * SKIP is 0xDEADD00D.
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MMAPPER_ALLOCATE_ADDRESS 330
#define SYS_MMAPPER_FREE_ADDRESS     331

#define FLAG_1M   0x400
#define AREA      0x10000000ULL
#define SKIP      0xDEADD00DULL
#define TRIES     8

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000

#define WORDS   34
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static gcmContextData *context;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = PAYLOAD;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)out, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)out, len);
}

/* Reserve up to TRIES areas, record them at base_slot / rc_slot, then
 * free every one it got. */
static void reserve_all(unsigned base_slot, unsigned rc_slot)
{
    u64 got[TRIES];
    unsigned n = 0, i;
    for (i = 0; i < TRIES; i++) {
        put(base_slot + i, 0);
        put(rc_slot + i, SKIP);
    }
    for (i = 0; i < TRIES; i++) {
        u64 rc;
        w32 = 0;
        rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_1M, AREA, EA(&w32));
        put(rc_slot + i, rc);
        if (rc != 0)
            break;
        got[n++] = w32;
        put(base_slot + i, w32);
    }
    for (i = 0; i < n; i++)
        sc(SYS_MMAPPER_FREE_ADDRESS, got[i], 0, 0, 0);
}

int main(void)
{
    sys_mem_addr_t host = 0;
    s32 rc;

    put(0, ~0ULL);
    put(1, 0);
    reserve_all(2, 10);

    rc = sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host);
    if (rc == 0)
        rc = rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    put(0, (u64)(u32)rc);
    if (rc == 0)
        put(1, (u64)(unsigned long)gcmGetControlRegister());

    reserve_all(18, 26);
    write_frame();
    return 0;
}
