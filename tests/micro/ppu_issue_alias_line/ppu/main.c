/* PPU probe: the cost of a load through a second effective address of a
 * line the data cache holds.
 *
 * The probe allocates one 256 MiB address area A (330) and one 1 MiB
 * segment M (332), and maps M twice with search-and-map (337): at X and
 * then at Y. At the end it unmaps Y and X (335) and frees M (329) and A
 * (331).
 *
 * Two rings of 2N doubleword pointers over N = 192 lines of M (24 KB),
 * each visited by a dependent `ld` chase:
 *   plain  at M + 0x10000: element j is X + 0x10000 + 128 (j mod N),
 *          plus 8 when j >= N.
 *   alias  at M + 0x30000: element j is X + 0x30000 + 128 (j mod N) for
 *          j < N, and Y + 0x30000 + 128 (j mod N) + 8 for j >= N.
 * Element j holds element j + 1 (mod 2N). Each lap of a ring loads every
 * line twice; the alias ring loads it once through X and once through Y.
 * N lines fit the 32 KB data cache; 2N lines do not.
 *
 * Each run is one untimed lap, then LAPS timed laps between two time
 * base reads: 2N LAPS loads, eight to a `bdnz` iteration. REPS runs of
 * each ring, plain then alias in each repetition.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2  REPS                     3  timed loads a run (2N LAPS)
 *   4  X                        5  Y
 *   6..10   the plain ring's time base ticks, one a run
 *  11..15   the alias ring's time base ticks, one a run
 *  16  unmap X rc  17  unmap Y rc  18  free M rc  19  free A rc
 *  20  the plain chase ended where it began in every run (1 = yes)
 *  21  the alias chase ended where it began in every run (1 = yes)
 * Steps: 1 the area, 2 the segment, 3 the map at X, 4 the map at Y,
 * 5 Y equal to X (its code is 0). A teardown word holds SKIP when its
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

#define WORDS  22
#define REPS   5
#define LINES  192
#define LAPS   64
#define PLAIN  0x10000ul
#define ALIAS  0x30000ul
/* Ring elements a lap, and `bdnz` iterations of eight loads a lap. */
#define ELEMENTS (2 * LINES)
#define ITERS_A_LAP (ELEMENTS / 8)

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

/* Element j of the ring at offset `off`, its second half through `high`. */
static unsigned long element(unsigned long x, unsigned long high, unsigned long off,
                             unsigned int j)
{
    j %= ELEMENTS;
    if (j < LINES)
        return x + off + 128ul * j;
    return high + off + 128ul * (j - LINES) + 8;
}

/* Writes the ring through X: element j holds element j + 1. */
static void build(unsigned long x, unsigned long high, unsigned long off)
{
    unsigned int j;
    for (j = 0; j < ELEMENTS; j++) {
        unsigned long at = element(x, x, off, j);
        *(volatile unsigned long long *)at = element(x, high, off, j + 1);
    }
    __sync_synchronize();
}

/* `iters` iterations of eight dependent loads from `*p`; returns the time
 * base ticks they took and leaves the last pointer in `*p`. */
static unsigned int chase(unsigned long *p, unsigned long iters)
{
    unsigned long q = *p;
    unsigned long long t0, t1;
    __asm__ volatile ("mftb %0" : "=r"(t0) : : "memory");
    __asm__ volatile ("mtctr %1\n"
                      "1:\n\t"
                      "ld %0,0(%0)\n\tld %0,0(%0)\n\tld %0,0(%0)\n\tld %0,0(%0)\n\t"
                      "ld %0,0(%0)\n\tld %0,0(%0)\n\tld %0,0(%0)\n\tld %0,0(%0)\n\t"
                      "bdnz 1b"
                      : "+b"(q) : "r"(iters) : "ctr", "memory");
    __asm__ volatile ("mftb %0" : "=r"(t1) : : "memory");
    *p = q;
    return (unsigned int)(t1 - t0);
}

/* One run of the ring that starts at `start`: an untimed lap, then LAPS
 * timed laps. Clears `*ok` when a chase does not end at `start`. */
static unsigned int run(unsigned long start, unsigned int *ok)
{
    unsigned long p = start;
    unsigned int ticks;
    chase(&p, ITERS_A_LAP);
    if (p != start)
        *ok = 0;
    ticks = chase(&p, (unsigned long)ITERS_A_LAP * LAPS);
    if (p != start)
        *ok = 0;
    return ticks;
}

int main(void)
{
    unsigned int r, plain_ok = 1, alias_ok = 1;
    u64 rc, a = 0, m = 0, x = 0, y = 0;
    int have_a = 0, have_m = 0, have_x = 0, have_y = 0;

    memset((void *)result, 0, sizeof(result));
    result[2] = REPS;
    result[3] = ELEMENTS * LAPS;

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

    build((unsigned long)x, (unsigned long)x, PLAIN);
    build((unsigned long)x, (unsigned long)y, ALIAS);
    for (r = 0; r < REPS; r++) {
        result[6 + r] = run((unsigned long)x + PLAIN, &plain_ok);
        result[11 + r] = run((unsigned long)x + ALIAS, &alias_ok);
    }
    result[20] = plain_ok;
    result[21] = alias_ok;

teardown:
    result[16] = have_x ? unmap(x) : SKIP;
    result[17] = have_y ? unmap(y) : SKIP;
    result[18] = have_m ? (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0) : SKIP;
    result[19] = have_a ? (unsigned int)sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0) : SKIP;
    write_frame();
    return 0;
}
