/* PPU probe: one shared-memory segment mapped at two addresses.
 *
 * The probe allocates one 256 MiB address area A (330) and one 1 MiB
 * segment M (332), and maps M twice with search-and-map (337): at X and
 * then at Y. Every byte of M is then at two effective addresses. At the
 * end it unmaps Y and X (335) and frees M (329) and A (331).
 *
 * Case c works on the doubleword at offset LINE(c) of M, a line no
 * other case touches. Every trial first writes SENT there through X with
 * an ordinary store and a sync, then runs the case in one asm block.
 * TRIALS trials per case.
 *   0  lwz Y; stw v X; lwz Y
 *        first count: the second load read v
 *        second count: the first load read SENT
 *   1  lwarx X; stwcx. v Y      2  lwarx Y; stwcx. v X
 *   3  ldarx X; stdcx. v Y      4  lwarx X; stwcx. v X (control)
 *        first count: CR0[EQ] set
 *        second count: the store-conditional's word, read through the
 *        other mapping, holds v
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  TRIALS
 *   3  A                        4  X                 5  Y
 *   6 + 2c, 7 + 2c  case c's two counts
 *  16  unmap X rc  17  unmap Y rc  18  free M rc  19  free A rc
 * Steps: 1 the area, 2 the segment, 3 the map at X, 4 the map at Y,
 * 5 Y equal to X (its code is 0). A failed step leaves every count 0.
 * A teardown word holds SKIP when its object was never made.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MMAPPER_ALLOCATE_ADDRESS 330
#define SYS_MMAPPER_FREE_ADDRESS     331
#define SYS_MMAPPER_ALLOCATE_SHM     332
#define SYS_MMAPPER_FREE_SHM         329
#define SYS_MMAPPER_UNMAP_SHM        335
#define SYS_MMAPPER_SEARCH_AND_MAP   337

#define KEYLESS   0xFFFF000000000000ULL
#define FLAG_1M   0x400
#define PROT_RW   0x40000
#define AREA      0x10000000ULL
#define MIB       0x100000ULL
#define SKIP      0xDEADD00Du

#define CASES  5
#define WORDS  20
#define TRIALS 32
#define SENT   0xA5A5A5A5A5A5A5A5ull
#define LINE(c) (0x1000ul + 0x100ul * (unsigned long)(c))

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
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

static unsigned int unmap(u64 addr)
{
    w32 = 0xFFFFFFFFu;
    return (unsigned int)sc(SYS_MMAPPER_UNMAP_SHM, addr, EA(&w32), 0, 0);
}

static void fill(unsigned long x, unsigned int c)
{
    *(volatile unsigned long long *)(x + LINE(c)) = SENT;
    __sync_synchronize();
}

/* The word at `p`, read with an ordinary load. */
static unsigned int word_at(unsigned long p)
{
    return *(volatile unsigned int *)p;
}

static unsigned long long dword_at(unsigned long p)
{
    return *(volatile unsigned long long *)p;
}

/* One trial of case `c`; adds its two counts. */
static void trial(unsigned int c, unsigned long x, unsigned long y, unsigned long long v)
{
    unsigned long px = x + LINE(c), py = y + LINE(c);
    unsigned long long t0 = 0, t1 = 0;
    unsigned int cr = 0, eq;
    switch (c) {
    case 0:
        __asm__ volatile ("lwz %0,0(%3)\n\tstw %4,0(%2)\n\tlwz %1,0(%3)"
                          : "=&r"(t0), "=&r"(t1) : "b"(px), "b"(py), "r"(v) : "memory");
        result[6] += (unsigned int)t1 == (unsigned int)v;
        result[7] += (unsigned int)t0 == (unsigned int)SENT;
        return;
    case 1:
        __asm__ volatile ("lwarx %1,0,%2\n\tstwcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t0) : "r"(px), "r"(py), "r"(v) : "cr0", "memory");
        break;
    case 2:
        __asm__ volatile ("lwarx %1,0,%2\n\tstwcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t0) : "r"(py), "r"(px), "r"(v) : "cr0", "memory");
        break;
    case 3:
        __asm__ volatile ("ldarx %1,0,%2\n\tstdcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t0) : "r"(px), "r"(py), "r"(v) : "cr0", "memory");
        break;
    default:
        __asm__ volatile ("lwarx %1,0,%2\n\tstwcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t0) : "r"(px), "r"(px), "r"(v) : "cr0", "memory");
        break;
    }
    eq = (cr >> 29) & 1;
    result[6 + 2 * c] += eq;
    switch (c) {
    case 1:
    case 4:
        result[7 + 2 * c] += word_at(px) == (unsigned int)v;
        break;
    case 2:
        result[7 + 2 * c] += word_at(py) == (unsigned int)v;
        break;
    default:
        result[7 + 2 * c] += dword_at(px) == v;
        break;
    }
}

int main(void)
{
    unsigned int c, i;
    u64 rc, a = 0, m = 0, x = 0, y = 0;
    int have_a = 0, have_m = 0, have_x = 0, have_y = 0;

    memset((void *)result, 0, sizeof(result));
    result[2] = TRIALS;

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_1M, AREA, EA(&w32));
    if (rc != 0) {
        result[0] = 1;
        result[1] = (unsigned int)rc;
        goto teardown;
    }
    a = w32;
    have_a = 1;
    result[3] = (unsigned int)a;

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, MIB, FLAG_1M, EA(&w32));
    if (rc != 0) {
        result[0] = 2;
        result[1] = (unsigned int)rc;
        goto teardown;
    }
    m = w32;
    have_m = 1;

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_SEARCH_AND_MAP, a, m, PROT_RW, EA(&w32));
    if (rc != 0) {
        result[0] = 3;
        result[1] = (unsigned int)rc;
        goto teardown;
    }
    x = w32;
    have_x = 1;
    result[4] = (unsigned int)x;

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_SEARCH_AND_MAP, a, m, PROT_RW, EA(&w32));
    if (rc != 0) {
        result[0] = 4;
        result[1] = (unsigned int)rc;
        goto teardown;
    }
    y = w32;
    have_y = 1;
    result[5] = (unsigned int)y;
    if (y == x) {
        result[0] = 5;
        result[1] = 0;
        goto teardown;
    }

    for (c = 0; c < CASES; c++) {
        for (i = 0; i < TRIALS; i++) {
            unsigned long long v = 0x1122334455667700ull + (c << 8) + i;
            fill((unsigned long)x, c);
            trial(c, (unsigned long)x, (unsigned long)y, v);
        }
    }

teardown:
    result[16] = have_x ? unmap(x) : SKIP;
    result[17] = have_y ? unmap(y) : SKIP;
    result[18] = have_m ? (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0) : SKIP;
    result[19] = have_a ? (unsigned int)sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0) : SKIP;
    write_frame();
    return 0;
}
