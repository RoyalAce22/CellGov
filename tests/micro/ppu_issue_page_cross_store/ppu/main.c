/* PPU probe: what a store to a line the caches do not hold costs before
 * a sync completes, through a store that crosses into a next page mapped
 * to another address's real page and through a plain store.
 *
 * The probe allocates one 256 MiB address area A (330) and one 1 MiB
 * segment M (332), and maps M twice with search-and-map (337): at X and
 * then at Y. It needs Y = X + 1 MiB, so the page after X's last page is
 * Y's first page, which maps to real page M + 0. At the end it unmaps X
 * and Y (335) and frees M (329) and A (331).
 *
 * P = X + 0xFFF80 is the start of X's last line. `std p,0x7C(p)` with
 * p = P writes M's last word (0) and M's first word (P): its second line
 * is the line of M + 0, which X + 0 also reaches. `stw p,0(x)` writes
 * M's first word (P) through X. Every store writes the bytes that are
 * already there.
 *
 * Cases (each a loop of ITER iterations; f is the flushed line):
 *   0  dcbf f; sync; std p,0x7C(p); sync                f = X + 0
 *   1  dcbf f; sync; std p,0x7C(p); sync                f = X + 0x80
 *   2  dcbf f; sync; stw p,0(x); sync                   f = X + 0
 *   3  dcbf f; sync; stw p,0(x); sync                   f = X + 0x80
 *   4  dcbf f; sync; stw p,0(x); sync; ld t,0(f)        f = X + 0
 *   5  dcbf f; sync; stw p,0(x); sync; ld t,0(f)        f = X + 0x80
 *
 * One untimed run of every case, then REPS repetitions, each one timed
 * run of every case in case order, between two time base reads.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2  REPS                     3  ITER
 *   4  X                        5  Y
 *   6 + 2c, 7 + 2c  case c's smallest and largest time base ticks a run
 *  18  unmap X rc  19  unmap Y rc  20  free M rc  21  free A rc
 *  22  M's first word is P and its last word 0 after the runs (1 = yes)
 *  23  0x50504353 ("PPCS"): the probe reached the teardown
 * Steps: 1 the area, 2 the segment, 3 the map at X, 4 the map at Y,
 * 5 Y not X + 1 MiB (its code is 0). A teardown word holds SKIP when its
 * object was never made.
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

#define WORDS  24
#define CASES  6
#define REPS   8
#define ITER   256
#define LAST   0xFFF80ul

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile unsigned long sink;

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

static inline unsigned long long timebase(void)
{
    unsigned long long tb;
    __asm__ volatile ("sync\n\tmftb %0" : "=r"(tb) : : "memory");
    return tb;
}

/* Cases 0 and 1: flush `f`, then the crossing store of P at P. */
static unsigned int __attribute__((noinline))
flush_cross(unsigned long f, unsigned long s)
{
    unsigned long long t0 = timebase();
    __asm__ volatile ("mtctr %2\n"
                      ".p2align 5\n"
                      "1:\n\t"
                      "dcbf 0,%0\n\t"
                      "sync\n\t"
                      "std %1,0x7C(%1)\n\t"
                      "sync\n\t"
                      "bdnz 1b"
                      : : "r"(f), "b"(s), "r"((unsigned long)ITER)
                      : "ctr", "memory");
    return (unsigned int)(timebase() - t0);
}

/* Cases 2 and 3: flush `f`, then a store of P's low word at `x`. */
static unsigned int __attribute__((noinline))
flush_plain(unsigned long f, unsigned long x, unsigned long v)
{
    unsigned long long t0 = timebase();
    __asm__ volatile ("mtctr %3\n"
                      ".p2align 5\n"
                      "1:\n\t"
                      "dcbf 0,%0\n\t"
                      "sync\n\t"
                      "stw %2,0(%1)\n\t"
                      "sync\n\t"
                      "bdnz 1b"
                      : : "r"(f), "b"(x), "r"(v), "r"((unsigned long)ITER)
                      : "ctr", "memory");
    return (unsigned int)(timebase() - t0);
}

/* Cases 4 and 5: flush `f`, a store of P's low word at `x`, then a load
 * of `f`. */
static unsigned int __attribute__((noinline))
flush_plain_load(unsigned long f, unsigned long x, unsigned long v)
{
    unsigned long t;
    unsigned long long t0 = timebase();
    __asm__ volatile ("mtctr %4\n"
                      ".p2align 5\n"
                      "1:\n\t"
                      "dcbf 0,%1\n\t"
                      "sync\n\t"
                      "stw %3,0(%2)\n\t"
                      "sync\n\t"
                      "ld %0,0(%1)\n\t"
                      "bdnz 1b"
                      : "=&r"(t) : "r"(f), "b"(x), "r"(v), "r"((unsigned long)ITER)
                      : "ctr", "memory");
    t0 = timebase() - t0;
    sink += t;
    return (unsigned int)t0;
}

static unsigned int run(unsigned int c, unsigned long x)
{
    unsigned long p = x + LAST;
    switch (c) {
    case 0: return flush_cross(x, p);
    case 1: return flush_cross(x + 0x80, p);
    case 2: return flush_plain(x, x, p);
    case 3: return flush_plain(x + 0x80, x, p);
    case 4: return flush_plain_load(x, x, p);
    default: return flush_plain_load(x + 0x80, x, p);
    }
}

int main(void)
{
    unsigned int c, r;
    u64 rc, a = 0, m = 0, x = 0, y = 0;
    int have_a = 0, have_m = 0, have_x = 0, have_y = 0;

    memset((void *)result, 0, sizeof(result));
    result[2] = REPS;
    result[3] = ITER;

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_1M, AREA, EA(&w32));
    if (rc != 0) {
        result[0] = 1;
        result[1] = (unsigned int)rc;
        goto teardown;
    }
    a = w32;
    have_a = 1;

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
    if (y != x + MIB) {
        result[0] = 5;
        result[1] = 0;
        goto teardown;
    }

    /* M's last word 0, M's first word P, both through X. */
    *(volatile unsigned int *)(unsigned long)(x + MIB - 4) = 0;
    *(volatile unsigned int *)(unsigned long)x = (unsigned int)(x + LAST);
    __sync_synchronize();

    for (c = 0; c < CASES; c++) {
        result[6 + 2 * c] = 0xFFFFFFFFu;
        run(c, (unsigned long)x);
    }
    for (r = 0; r < REPS; r++) {
        for (c = 0; c < CASES; c++) {
            unsigned int t = run(c, (unsigned long)x);
            if (t < result[6 + 2 * c])
                result[6 + 2 * c] = t;
            if (t > result[7 + 2 * c])
                result[7 + 2 * c] = t;
        }
    }
    result[22] = *(volatile unsigned int *)(unsigned long)x == (unsigned int)(x + LAST)
                 && *(volatile unsigned int *)(unsigned long)(x + MIB - 4) == 0;

teardown:
    result[18] = have_x ? unmap(x) : SKIP;
    result[19] = have_y ? unmap(y) : SKIP;
    result[20] = have_m ? (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0) : SKIP;
    result[21] = have_a ? (unsigned int)sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0) : SKIP;
    result[23] = 0x50504353u;
    write_frame();
    return 0;
}
