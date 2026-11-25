/* PPU probe: which line a touch brings in when the line's real page is
 * mapped at two effective addresses. One 1 MiB shared segment is mapped
 * at X and at Y. Each timed load reads a line through X after dcbf has
 * dropped the line from both caches, and after one of four cases:
 *
 *   case
 *   0  no touch
 *   1  dcbt   0,<line through X>
 *   2  dcbt   0,<line through Y>
 *   3  dcbtst 0,<line through Y>
 *
 * A wait of WAIT time base ticks after the touch lets a line that the
 * touch requests arrive before the load. The load is timed with mftb
 * around `ld; add` on its result, so the second mftb waits for the
 * load's data. A run is LINES such loads, one a line, over 128 KB of the
 * segment, and its time is the sum of the LINES time base deltas. Each
 * case runs REPS times, interleaved with the other cases.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2  REPS                     3  LINES
 *   4  X                        5  Y
 *   6..9    the smallest run of cases 0 to 3, in time base ticks
 *   10..13  the largest run of cases 0 to 3, in time base ticks
 *   14..17  teardown return codes: unmap X, unmap Y, free the segment,
 *           free the area
 *   18  every timed load read the word written through X (1 = yes)
 *   19  0x54434841 ("TCHA"): the probe wrote every word
 * Steps: 1 the area, 2 the segment, 3 the map at X, 4 the map at Y,
 * 5 Y equal to X (its code is 0). A teardown word holds SKIP when its
 * object was never made, and the run words stay 0 when a step failed.
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
#define END_MARK  0x54434841u
#define FILL      0x5A5AC3C300000000ULL

#define WORDS  20
#define REPS   5
#define CASES  4
#define LINES  1024
#define WAIT   200
#define LINE   128ul
#define OFF    0x10000ul

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

static inline unsigned long long tb(void)
{
    unsigned long long t;
    __asm__ volatile ("mftb %0" : "=r"(t) : : "memory");
    return t;
}

static inline void wait_ticks(unsigned long long n)
{
    unsigned long long t0 = tb();
    while (tb() - t0 < n)
        ;
}

/* One run of case `c`: returns the summed time base ticks of its LINES
 * timed loads and clears `*ok` when a load reads a word other than the
 * one written through X. */
static unsigned int run(unsigned long x, unsigned long y, unsigned int c, unsigned int *ok)
{
    unsigned int i, sum = 0;
    for (i = 0; i < LINES; i++) {
        unsigned long off = OFF + LINE * i;
        unsigned long px = x + off;
        unsigned long py = y + off;
        unsigned long long t0, t1, v, use;
        __asm__ volatile ("dcbf 0,%0\n\tsync" : : "r"(px) : "memory");
        if (c == 1)
            __asm__ volatile ("dcbt 0,%0" : : "r"(px) : "memory");
        else if (c == 2)
            __asm__ volatile ("dcbt 0,%0" : : "r"(py) : "memory");
        else if (c == 3)
            __asm__ volatile ("dcbtst 0,%0" : : "r"(py) : "memory");
        wait_ticks(WAIT);
        __asm__ volatile ("mftb %0\n\t"
                          "ld %1,0(%4)\n\t"
                          "add %2,%1,%1\n\t"
                          "mftb %3"
                          : "=&r"(t0), "=&r"(v), "=&r"(use), "=&r"(t1)
                          : "b"(px) : "memory");
        if (v != (FILL | off))
            *ok = 0;
        sum += (unsigned int)(t1 - t0);
    }
    return sum;
}

int main(void)
{
    unsigned int r, c, i, ok = 1;
    u64 rc, a = 0, m = 0, x = 0, y = 0;
    int have_a = 0, have_m = 0, have_x = 0, have_y = 0;

    memset((void *)result, 0, sizeof(result));
    result[2] = REPS;
    result[3] = LINES;

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
    if (y == x) {
        result[0] = 5;
        result[1] = 0;
        goto teardown;
    }

    for (i = 0; i < LINES; i++) {
        unsigned long off = OFF + LINE * i;
        *(volatile unsigned long long *)((unsigned long)x + off) = FILL | off;
    }
    __sync_synchronize();

    for (c = 0; c < CASES; c++) {
        result[6 + c] = 0xFFFFFFFFu;
        result[10 + c] = 0;
    }
    for (r = 0; r < REPS; r++) {
        for (c = 0; c < CASES; c++) {
            unsigned int t = run((unsigned long)x, (unsigned long)y, c, &ok);
            if (t < result[6 + c])
                result[6 + c] = t;
            if (t > result[10 + c])
                result[10 + c] = t;
        }
    }
    result[18] = ok;

teardown:
    result[14] = have_x ? unmap(x) : SKIP;
    result[15] = have_y ? unmap(y) : SKIP;
    result[16] = have_m ? (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0) : SKIP;
    result[17] = have_a ? (unsigned int)sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0) : SKIP;
    result[19] = END_MARK;
    write_frame();
    return 0;
}
