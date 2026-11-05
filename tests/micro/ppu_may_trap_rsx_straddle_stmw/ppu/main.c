/* PPU probe: what a word-aligned stmw does when its first eight bytes are
 * in cacheable memory and its last four in RSX local memory, once libgcm
 * has brought up an RSX context.
 *
 * A 1 MiB-page host block and rsxInit map local memory at 0xC0000000;
 * its first 256 bytes are filled with a pattern. The probe then reserves
 * 256 MiB address areas (330) until it gets the one at 0xB0000000, frees
 * the others, and maps a 1 MiB segment (332, 334) at 0xBFF00000, the top
 * of that area, whose last 16 bytes it fills with 0x10, 0x11, ...
 * One stmw r29 runs at 0xBFFFFFF8 with r29, r30 and r31 holding
 * 0x01010101, 0x02020202 and 0x03030303. It may end or stop the process
 * (the PPE refuses a store multiple in caching-inhibited storage), so a
 * progress record is written to the result file before it runs; the
 * final frame replaces it. A run that completes unmaps and
 * frees what it made.
 *
 * Payload (14 big-endian u64):
 *   0       DONE once the stmwran, PROG in the progress record or
 *           when the setup failed
 *   1       1 in the progress record (0 otherwise)
 *   2       rsxInit rc (0xFFFFFFFFFFFFFFFF when not sent)
 *   3       local memory address from gcmGetConfiguration
 *   4       330 rc of the last reserve sent
 *   5       the base that reserve wrote
 *   6       332 rc (SKIP when not sent)
 *   7       334 rc of the map at 0xBFF00000 (SKIP when not sent)
 *   8       stmw EA (SKIP when not sent)
 *   9       the u64 at 0xBFFFFFF8 after the stmw(SKIP when not sent)
 *   10      335 unmap rc (SKIP when not sent)
 *   11      329 free segment rc (SKIP when not sent)
 *   12      331 free area rc (SKIP when not sent)
 *   13      local memory's first u64 after the stmw(SKIP when not sent)
 *
 * The fill puts 0xA5A5A5A5_0000000i in u64 i of local memory's first
 * 256 bytes. SKIP is 0xDEADD00D.
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
#define SYS_MMAPPER_ALLOCATE_SHM     332
#define SYS_MMAPPER_FREE_SHM         329
#define SYS_MMAPPER_MAP_SHM          334
#define SYS_MMAPPER_UNMAP_SHM        335

#define KEYLESS   0xFFFF000000000000ULL
#define FLAG_1M   0x400
#define PROT_RW   0x40000
#define AREA      0x10000000ULL
#define MIB       0x100000ULL
#define WANT      0xB0000000ULL
#define TRIES     8

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000

#define DONE 0x444F4E45ULL
#define PROG 0x50524F47ULL
#define SKIP 0xDEADD00DULL
#define FILL 0xA5A5A5A500000000ULL

#define FILL_WORDS 32
#define WORDS   14
#define PAYLOAD (WORDS * 8)

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static gcmContextData *context;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
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
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

int main(void)
{
    sys_mem_addr_t host = 0;
    gcmConfiguration config;
    volatile u64 *local;
    volatile unsigned char *top;
    u64 got[TRIES];
    u64 rc, area = 0, seg = 0;
    unsigned long ea;
    unsigned n = 0, i;

    result[0] = PROG;
    result[1] = 0;
    result[2] = ~0ULL;
    for (i = 3; i < WORDS; i++)
        result[i] = SKIP;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        write_frame();
        return 1;
    }
    result[2] = (u64)(s64)rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    if (result[2] != 0) {
        write_frame();
        return 1;
    }
    gcmGetConfiguration(&config);
    local = (volatile u64 *)config.localAddress;
    result[3] = (u64)(unsigned long)local;
    for (i = 0; i < FILL_WORDS; i++)
        local[i] = FILL | i;

    /* Reserve areas until the one at WANT, keeping it and freeing the rest. */
    for (i = 0; i < TRIES; i++) {
        w32 = 0;
        rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_1M, AREA, EA(&w32));
        result[4] = rc;
        result[5] = w32;
        if (rc != 0)
            break;
        if (w32 == WANT) {
            area = WANT;
            break;
        }
        got[n++] = w32;
    }
    for (i = 0; i < n; i++)
        sc(SYS_MMAPPER_FREE_ADDRESS, got[i], 0, 0, 0);
    if (area == 0) {
        write_frame();
        return 1;
    }

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, MIB, FLAG_1M, EA(&w32));
    result[6] = rc;
    if (rc != 0) {
        result[12] = sc(SYS_MMAPPER_FREE_ADDRESS, area, 0, 0, 0);
        write_frame();
        return 1;
    }
    seg = w32;
    rc = sc(SYS_MMAPPER_MAP_SHM, area + AREA - MIB, seg, PROT_RW, 0);
    result[7] = rc;
    if (rc != 0) {
        result[11] = sc(SYS_MMAPPER_FREE_SHM, seg, 0, 0, 0);
        result[12] = sc(SYS_MMAPPER_FREE_ADDRESS, area, 0, 0, 0);
        write_frame();
        return 1;
    }
    top = (volatile unsigned char *)(unsigned long)(area + AREA - 16);
    for (i = 0; i < 16; i++)
        top[i] = (unsigned char)(0x10 + i);

    ea = (unsigned long)(area + AREA - 8);
    result[8] = ea;
    result[1] = 1;
    CGOV_OUT_FILE_WRITE((const void *)result, PAYLOAD);
    {
        register unsigned long r29 __asm__("r29") = 0x01010101UL;
        register unsigned long r30 __asm__("r30") = 0x02020202UL;
        register unsigned long r31 __asm__("r31") = 0x03030303UL;
        __asm__ volatile("stmw 29,0(%3)"
                         : : "r"(r29), "r"(r30), "r"(r31), "b"(ea) : "memory");
    }
    result[9] = *(volatile u64 *)(unsigned long)(area + AREA - 8);
    result[13] = local[0];

    w32 = 0xFFFFFFFFu;
    result[10] = sc(SYS_MMAPPER_UNMAP_SHM, area + AREA - MIB, EA(&w32), 0, 0);
    result[11] = sc(SYS_MMAPPER_FREE_SHM, seg, 0, 0, 0);
    result[12] = sc(SYS_MMAPPER_FREE_ADDRESS, area, 0, 0, 0);
    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
