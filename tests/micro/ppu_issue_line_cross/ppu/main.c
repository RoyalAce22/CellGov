/* PPU probe: what a load or a load or store multiple costs when its bytes
 * cross from one 128-byte line into the next, with the second line in the
 * L1 or in memory. One thread; no LV2 call before the result frame.
 *
 * Each loop gets its own 32 KB slot of `pool`, made of 256-byte nodes:
 * two lines a node, the node's base at its first line. Each node holds the
 * address of the next node at the loop's offset:
 *   - an ld loop stores it as a big-endian doubleword at the offset;
 *   - an lmw loop stores it as a big-endian word at the offset plus 12,
 *     which `lmw 28,off` loads into r31.
 * A chase loop is `ld p,off(p)` or `lmw 28,off(p); mr p,31`, then bdnz,
 * so each access waits for the one before it.
 *
 * A warm loop chases a 64-node ring (16 KB) 1 << 14 times; its earlier
 * runs leave both lines of each node in the L1. A cold loop chases the
 * first 96 nodes (24 KB) once. Before each run of a cold loop, the probe:
 *   - flushes (dcbf) each node's second line, and for a both-cold loop its
 *     first line too;
 *   - runs sync;
 *   - otherwise loads one byte of each node's first line, so the first
 *     lines are in the L1, then runs sync again.
 * The store loops run `stmw 28,off(p)` at one fixed node 1 << 16 times.
 *
 *   loop  access                 offset  bytes        iterations
 *   0     warm ld                112     one sector   1 << 14
 *   1     warm ld                116     one sector, word aligned
 *   2     warm ld                92      two 32-byte sectors of one line
 *   3     warm ld                124     two lines
 *   4     ld, second line cold   112     one line     96
 *   5     ld, second line cold   124     two lines    96
 *   6     ld, both lines cold    112     one line     96
 *   7     ld, both lines cold    124     two lines    96
 *   8     warm lmw r28           96      one line     1 << 14
 *   9     warm lmw r28           120     two lines    1 << 14
 *   10    lmw r28, second cold   96      one line     96
 *   11    lmw r28, second cold   120     two lines    96
 *   12    warm stmw r28          96      one line     1 << 16
 *   13    warm stmw r28          120     two lines    1 << 16
 *
 * Every offset is a multiple of 4: a misaligned lmw or stmw ends the
 * process on the console.
 *
 * Payload: big-endian u64 words. For loop L:
 *   2L, 2L + 1   the smallest and the largest time-base delta of REPS runs
 *   28 + L       the effective address of the loop's slot
 *   42 + L       the loop's iterations a run
 * Word 56: the sum of every chase's final pointer (keeps each loop live).
 * Word 57: REPS. Word 58: 0x50504C58 ("PPLX"): the probe wrote every word.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS   14
#define WORDS   59
#define PAYLOAD (WORDS * 8)
#define REPS    8

#define LINE        128
#define NODE        256
#define SLOT        (32 * 1024)
#define WARM_NODES  64
#define COLD_NODES  96
#define WARM_ITER   (1u << 14)
#define STORE_ITER  (1u << 16)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;
static unsigned char pool[LOOPS][SLOT] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static inline u64 timebase(void)
{
    u64 tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
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

static void put_be(unsigned char *at, u64 v, unsigned bytes)
{
    unsigned i;
    for (i = 0; i < bytes; i++)
        at[i] = (unsigned char)(v >> (8 * (bytes - 1 - i)));
}

/* Link `nodes` nodes of `slot` into a ring: each node holds the next
 * node's address in `bytes` bytes at `at`. */
static void link_ring(unsigned char *slot, unsigned nodes, unsigned at, unsigned bytes)
{
    unsigned i;
    for (i = 0; i < nodes; i++)
        put_be(slot + i * NODE + at, EA(slot + ((i + 1) % nodes) * NODE), bytes);
}

/* Flush each node's second line, and with `first_cold` its first line;
 * otherwise bring each first line into the L1. */
static void __attribute__((noinline)) prepare(unsigned char *slot, int first_cold)
{
    unsigned i;
    unsigned acc = 0;
    for (i = 0; i < COLD_NODES; i++) {
        unsigned char *node = slot + i * NODE;
        __asm__ volatile ("dcbf 0,%0" : : "r"(node + LINE) : "memory");
        if (first_cold)
            __asm__ volatile ("dcbf 0,%0" : : "r"(node) : "memory");
    }
    __asm__ volatile ("sync" : : : "memory");
    if (first_cold)
        return;
    for (i = 0; i < COLD_NODES; i++)
        acc += *(volatile unsigned char *)(slot + i * NODE);
    __asm__ volatile ("sync" : : : "memory");
    sink += acc;
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"
#define CLOBBER_R28 "r28", "r29", "r30", "r31"

#define CHASE_LD(name, off)                                        \
    static u64 __attribute__((noinline)) name(u64 p, u64 n)        \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN "ld %[p]," #off "(%[p])\n" LOOP_END         \
            : [p] "+b"(p) : [n] "r"(n) : "ctr", "memory");         \
        t0 = timebase() - t0;                                      \
        sink += p;                                                 \
        return t0;                                                 \
    }

#define CHASE_LMW(name, off)                                       \
    static u64 __attribute__((noinline)) name(u64 p, u64 n)        \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN "lmw 28," #off "(%[p])\n" "mr %[p],31\n"    \
            LOOP_END                                               \
            : [p] "+b"(p) : [n] "r"(n)                             \
            : CLOBBER_R28, "ctr", "memory");                       \
        t0 = timebase() - t0;                                      \
        sink += p;                                                 \
        return t0;                                                 \
    }

#define STORE_STMW(name, off)                                      \
    static u64 __attribute__((noinline)) name(u64 p, u64 n)        \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN "stmw 28," #off "(%[p])\n" LOOP_END         \
            : : [p] "b"(p), [n] "r"(n) : "ctr", "memory");         \
        return timebase() - t0;                                    \
    }

CHASE_LD(ld_112, 112)
CHASE_LD(ld_116, 116)
CHASE_LD(ld_92, 92)
CHASE_LD(ld_124, 124)
CHASE_LMW(lmw_96, 96)
CHASE_LMW(lmw_120, 120)
STORE_STMW(stmw_96, 96)
STORE_STMW(stmw_120, 120)

/* Each loop's kernel, offset and kind. */
enum kind { WARM, SECOND_COLD, BOTH_COLD, STORE };

struct loop {
    u64 (*run)(u64, u64);
    unsigned off;
    unsigned lmw;
    enum kind kind;
};

static const struct loop LOOP[LOOPS] = {
    { ld_112, 112, 0, WARM },
    { ld_116, 116, 0, WARM },
    { ld_92, 92, 0, WARM },
    { ld_124, 124, 0, WARM },
    { ld_112, 112, 0, SECOND_COLD },
    { ld_124, 124, 0, SECOND_COLD },
    { ld_112, 112, 0, BOTH_COLD },
    { ld_124, 124, 0, BOTH_COLD },
    { lmw_96, 96, 1, WARM },
    { lmw_120, 120, 1, WARM },
    { lmw_96, 96, 1, SECOND_COLD },
    { lmw_120, 120, 1, SECOND_COLD },
    { stmw_96, 96, 1, STORE },
    { stmw_120, 120, 1, STORE },
};

static unsigned iterations(enum kind kind)
{
    switch (kind) {
    case WARM: return WARM_ITER;
    case STORE: return STORE_ITER;
    default: return COLD_NODES;
    }
}

int main(void)
{
    unsigned i, l, r;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (l = 0; l < LOOPS; l++) {
        const struct loop *lp = &LOOP[l];
        unsigned char *slot = pool[l];
        unsigned nodes = lp->kind == WARM ? WARM_NODES : COLD_NODES;
        unsigned n = iterations(lp->kind);
        u64 lo = ~0ull, hi = 0;
        if (lp->kind != STORE) {
            if (lp->lmw)
                link_ring(slot, nodes, lp->off + 12, 4);
            else
                link_ring(slot, nodes, lp->off, 8);
        }
        for (r = 0; r < REPS; r++) {
            u64 t;
            if (lp->kind == SECOND_COLD || lp->kind == BOTH_COLD)
                prepare(slot, lp->kind == BOTH_COLD);
            t = lp->run(EA(slot), n);
            if (t < lo)
                lo = t;
            if (t > hi)
                hi = t;
        }
        put(2 * l, lo);
        put(2 * l + 1, hi);
        put(28 + l, EA(slot));
        put(42 + l, n);
    }
    put(56, sink);
    put(57, REPS);
    put(58, 0x50504C58ull);

    write_frame();
    return 0;
}
