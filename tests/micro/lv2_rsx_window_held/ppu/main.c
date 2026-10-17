/* PPU program: where libgcm's RSX context window opens when the game
 * process already holds the 256 MiB granule an unheld window takes.
 *
 * Two 330 reserves run first and stay held, so the lowest two granules
 * are taken before any context opens. A 1 MiB-page host block and
 * rsxInit follow; libgcm's control register names the window's base.
 * Then 330 reserves until refused, recording where the remaining
 * granules are, and every area is freed.
 *
 * Payload (24 big-endian u64):
 *   0, 1    first held area: r3, base (0 on failure)
 *   2, 3    second held area: r3, base
 *   4, 5    host block (348, 1 MiB pages): r3, address
 *   6       rsxInit rc (0xFFFFFFFFFFFFFFFF when not sent)
 *   7       libgcm's control register address (0 when rsxInit fails)
 *   8..15   bases granted after rsxInit (0 for a reserve that failed or
 *           was not sent)
 *   16..23  r3 per reserve after rsxInit (SKIP when not sent)
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

#define WORDS   24
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

/* One 330; records r3 and the base at slot, slot + 1. */
static u64 reserve(unsigned slot)
{
    u64 rc;
    w32 = 0;
    rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_1M, AREA, EA(&w32));
    put(slot, rc);
    put(slot + 1, rc == 0 ? w32 : 0);
    return rc == 0 ? w32 : 0;
}

int main(void)
{
    sys_mem_addr_t host = 0;
    u64 held0, held1, got[TRIES];
    unsigned n = 0, i;
    s32 rc;

    put(6, ~0ULL);
    put(7, 0);
    for (i = 0; i < TRIES; i++) {
        put(8 + i, 0);
        put(16 + i, SKIP);
    }

    held0 = reserve(0);
    held1 = reserve(2);

    rc = sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host);
    put(4, (u64)(u32)rc);
    put(5, rc == 0 ? (u64)host : 0);
    if (rc == 0) {
        rc = rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
        put(6, (u64)(u32)rc);
        if (rc == 0)
            put(7, (u64)(unsigned long)gcmGetControlRegister());
    }

    for (i = 0; i < TRIES; i++) {
        u64 r;
        w32 = 0;
        r = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_1M, AREA, EA(&w32));
        put(16 + i, r);
        if (r != 0)
            break;
        got[n++] = w32;
        put(8 + i, w32);
    }
    for (i = 0; i < n; i++)
        sc(SYS_MMAPPER_FREE_ADDRESS, got[i], 0, 0, 0);
    if (held1 != 0)
        sc(SYS_MMAPPER_FREE_ADDRESS, held1, 0, 0, 0);
    if (held0 != 0)
        sc(SYS_MMAPPER_FREE_ADDRESS, held0, 0, 0, 0);

    write_frame();
    return 0;
}
