/* PPU probe: which line a load or store prices for its bytes past a
 * page boundary, when the next page maps to a real page that does not
 * follow the first page's.
 *
 * The probe allocates one 256 MiB address area A (330) and one 1 MiB
 * segment M (332), and maps M twice with search-and-map (337): at X and
 * then at Y. It needs Y = X + 1 MiB, so the page after X's last page is
 * Y's first page, which maps to real page M + 0. At
 * the end it unmaps X and Y (335) and frees M (329) and A (331).
 *
 * P = X + 0xFFF80 is the start of X's last line. The doubleword at
 * P + 0x7C covers the last 4 bytes of X and the first 4 bytes of Y: M's
 * last word (0) and M's first word (P). So `ld p,0x7C(p)` with p = P
 * reads P back, and its second line is the line of M + 0, which X + 0
 * also reaches.
 *
 * Cases (each a loop of ITER iterations; f is the flushed line):
 *   0  dcbf f; sync; ld p,0x7C(p)   f = X + 0     (M + 0 through X)
 *   1  dcbf f; sync; ld p,0x7C(p)   f = Y + 0     (M + 0 through Y)
 *   2  dcbf f; sync; ld p,0x7C(p)   f = X + 0x80  (a line no load reaches)
 *   3  dcbf f; sync; ld p,0x7C(p)   f = P         (the load's first line)
 *   4  sync; ld p,0x7C(p)
 *   5  dcbf f; sync; std p,0x7C(p); sync; ld t,0(f)   f = X + 0
 *   6  dcbf f; sync; std p,0x7C(p); sync; ld t,0(f)   f = X + 0x80
 * The stores write the bytes that are already there.
 *
 * One untimed run of every case, then REPS repetitions, each one timed
 * run of every case in case order, between two time base reads.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2  REPS                     3  ITER
 *   4  X                        5  Y
 *   6 + 2c, 7 + 2c  case c's smallest and largest time base ticks a run
 *  20  unmap X rc  21  unmap Y rc  22  free M rc  23  free A rc
 *  24  every load case's chase ended at P (1 = yes)
 *  25  0x50504341 ("PPCA"): the probe reached the teardown
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

#define WORDS  26
#define CASES  7
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

/* Cases 0 to 3: flush `f`, then the crossing load. */
static unsigned int __attribute__((noinline))
flush_load(unsigned long f, unsigned long *p)
{
    unsigned long q = *p;
    unsigned long long t0 = timebase();
    __asm__ volatile ("mtctr %2\n"
                      ".p2align 5\n"
                      "1:\n\t"
                      "dcbf 0,%1\n\t"
                      "sync\n\t"
                      "ld %0,0x7C(%0)\n\t"
                      "bdnz 1b"
                      : "+b"(q) : "r"(f), "r"((unsigned long)ITER)
                      : "ctr", "memory");
    t0 = timebase() - t0;
    *p = q;
    return (unsigned int)t0;
}

/* Case 4: the crossing load alone. */
static unsigned int __attribute__((noinline)) sync_load(unsigned long *p)
{
    unsigned long q = *p;
    unsigned long long t0 = timebase();
    __asm__ volatile ("mtctr %1\n"
                      ".p2align 5\n"
                      "1:\n\t"
                      "sync\n\t"
                      "ld %0,0x7C(%0)\n\t"
                      "bdnz 1b"
                      : "+b"(q) : "r"((unsigned long)ITER)
                      : "ctr", "memory");
    t0 = timebase() - t0;
    *p = q;
    return (unsigned int)t0;
}

/* Cases 5 and 6: flush `f`, the crossing store of P at P, then a load of
 * `f`. */
static unsigned int __attribute__((noinline))
flush_store(unsigned long f, unsigned long s)
{
    unsigned long t;
    unsigned long long t0 = timebase();
    __asm__ volatile ("mtctr %3\n"
                      ".p2align 5\n"
                      "1:\n\t"
                      "dcbf 0,%1\n\t"
                      "sync\n\t"
                      "std %2,0x7C(%2)\n\t"
                      "sync\n\t"
                      "ld %0,0(%1)\n\t"
                      "bdnz 1b"
                      : "=&r"(t) : "r"(f), "b"(s), "r"((unsigned long)ITER)
                      : "ctr", "memory");
    t0 = timebase() - t0;
    sink += t;
    return (unsigned int)t0;
}

/* One run of case `c`. Clears `*ok` when a load case does not end at P. */
static unsigned int run(unsigned int c, unsigned long x, unsigned long y,
                        unsigned int *ok)
{
    unsigned long start = x + LAST;
    unsigned long p = start;
    unsigned int ticks;
    switch (c) {
    case 0: ticks = flush_load(x, &p); break;
    case 1: ticks = flush_load(y, &p); break;
    case 2: ticks = flush_load(x + 0x80, &p); break;
    case 3: ticks = flush_load(start, &p); break;
    case 4: ticks = sync_load(&p); break;
    case 5: return flush_store(x, start);
    default: return flush_store(x + 0x80, start);
    }
    if (p != start)
        *ok = 0;
    return ticks;
}

int main(void)
{
    unsigned int c, r, ok = 1;
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
        run(c, (unsigned long)x, (unsigned long)y, &ok);
    }
    for (r = 0; r < REPS; r++) {
        for (c = 0; c < CASES; c++) {
            unsigned int t = run(c, (unsigned long)x, (unsigned long)y, &ok);
            if (t < result[6 + 2 * c])
                result[6 + 2 * c] = t;
            if (t > result[7 + 2 * c])
                result[7 + 2 * c] = t;
        }
    }
    result[24] = ok;

teardown:
    result[20] = have_x ? unmap(x) : SKIP;
    result[21] = have_y ? unmap(y) : SKIP;
    result[22] = have_m ? (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0) : SKIP;
    result[23] = have_a ? (unsigned int)sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0) : SKIP;
    result[25] = 0x50504341u;
    write_frame();
    return 0;
}
