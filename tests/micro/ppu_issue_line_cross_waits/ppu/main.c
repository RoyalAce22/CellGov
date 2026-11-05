/* PPU probe: what an access whose bytes cross from one 128-byte line into
 * the next leaves in, or waits for in, its second line. One thread; no
 * LV2 call before the result frame.
 *
 * A node is 256 bytes: two lines, line A at the node's base and line B
 * after it. Each node holds the address of the next node in its ring as a
 * big-endian doubleword at the loop's chase offset: 112 (in A), 124 (four
 * bytes in A, four in B) or 192 (in B).
 *
 * Store loops 0 to 2 and zero-count loops 11 to 13 each get their own
 * 32 KB slot and chase its first 96 nodes once a run. Before each run the
 * probe flushes (dcbf) both lines of each node, runs sync, runs the loop's
 * step on each node, and runs sync again. The steps:
 *   - stmw 28,off(node): 16 bytes at off;
 *   - lswx 28,node,r with r and the XER byte count 0;
 *   - lbz of the node's first byte;
 *   - nothing.
 *
 * Flush loops 3 to 6 each chase a 64-node ring (16 KB) in their own slot,
 * 1 << 12 times a run, with `addi r10,p,a; dcbf 0,r10; ld p,off(p)`: a is
 * 0 for line A and 128 for line B.
 *
 * Touch loops 7 to 10 chase one shared 8 MB ring, 1 << 12 times a run,
 * each loop's pointer carrying over from run to run, with
 * `addi r10,p,a; dcbt 0,r10; addi r11,p,b; dcbt 0,r11; ld p,off(p)`: a and
 * b are 2048 for line A eight nodes ahead and 2176 for its line B.
 *
 *   loop  step or body                    chase
 *   0     stmw at 96 (A only)             ld 192, B
 *   1     stmw at 120 (8 bytes into B)    ld 192, B
 *   2     stmw at 128 (B only)            ld 192, B
 *   3     dcbf B                          ld 124, A and B
 *   4     dcbf A                          ld 124, A and B
 *   5     dcbf B                          ld 112, A
 *   6     dcbf A                          ld 112, A
 *   7     dcbt A ahead, dcbt B ahead      ld 124, A and B
 *   8     dcbt A ahead twice              ld 124, A and B
 *   9     dcbt B ahead twice              ld 124, A and B
 *   10    dcbt A ahead, dcbt B ahead      ld 112, A
 *   11    lswx count 0 at the node        ld 112, A
 *   12    lbz at the node                 ld 112, A
 *   13    nothing                         ld 112, A
 *
 * Payload: big-endian u64 words. For loop L:
 *   2L, 2L + 1   the smallest and the largest time-base delta of REPS runs
 *   28 + L       the effective address of the loop's slot or ring
 *   42 + L       the loop's iterations a run
 * Word 56: the sum of every chase's final pointer (keeps each loop live).
 * Word 57: REPS. Word 58: 0x50504C57 ("PPLW"): the probe wrote every word.
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
#define SLOTS       10
#define RING_BYTES  (8u * 1024u * 1024u)
#define RING_NODES  (RING_BYTES / NODE)
#define WARM_NODES  64
#define COLD_NODES  96
#define WARM_ITER   (1u << 12)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;
static unsigned char pool[SLOTS][SLOT] __attribute__((aligned(128)));
static unsigned char ring[RING_BYTES] __attribute__((aligned(128)));
static u64 head124;
static u64 head112;

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

static void put_be64(unsigned char *at, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        at[i] = (unsigned char)(v >> (8 * (7 - i)));
}

/* Link `nodes` nodes at `base` into a ring: each node holds the next
 * node's address as a doubleword at `at`. */
static void link_ring(unsigned char *base, unsigned nodes, unsigned at)
{
    unsigned i;
    for (i = 0; i < nodes; i++)
        put_be64(base + i * NODE + at, EA(base + ((i + 1) % nodes) * NODE));
}

/* The per-node steps a cold loop's preparation runs. */
enum step { STMW_96, STMW_120, STMW_128, LSWX_0, LBZ, NONE };

/* Flush both lines of each node, sync, run `step` on each node, sync. */
static void __attribute__((noinline)) prepare(unsigned char *slot, enum step step)
{
    unsigned i;
    unsigned acc = 0;
    for (i = 0; i < COLD_NODES; i++) {
        unsigned char *node = slot + i * NODE;
        __asm__ volatile ("dcbf 0,%0" : : "r"(node + LINE) : "memory");
        __asm__ volatile ("dcbf 0,%0" : : "r"(node) : "memory");
    }
    __asm__ volatile ("sync" : : : "memory");
    for (i = 0; i < COLD_NODES; i++) {
        unsigned char *node = slot + i * NODE;
        switch (step) {
        case STMW_96:
            __asm__ volatile ("stmw 28,96(%0)" : : "b"(node) : "memory");
            break;
        case STMW_120:
            __asm__ volatile ("stmw 28,120(%0)" : : "b"(node) : "memory");
            break;
        case STMW_128:
            __asm__ volatile ("stmw 28,128(%0)" : : "b"(node) : "memory");
            break;
        case LSWX_0: {
            /* Neither address register is in r28 to r31: the console
             * ends a count-0 lswx whose RT is its RA. */
            u64 zero = 0;
            __asm__ volatile ("mtxer %1\n" "lswx 28,%0,%1\n"
                              : : "b"(node), "r"(zero)
                              : "r28", "r29", "r30", "r31", "xer", "memory");
            break;
        }
        case LBZ:
            acc += *(volatile unsigned char *)node;
            break;
        case NONE:
            break;
        }
    }
    __asm__ volatile ("sync" : : : "memory");
    sink += acc;
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define CHASE(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 *head, u64 n)    \
    {                                                              \
        u64 p = *head;                                             \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : [p] "+b"(p) : [n] "r"(n)                             \
            : "r10", "r11", "ctr", "memory");                      \
        t0 = timebase() - t0;                                      \
        *head = p;                                                 \
        sink += p;                                                 \
        return t0;                                                 \
    }

CHASE(ld_192, "ld %[p],192(%[p])\n")
CHASE(ld_112, "ld %[p],112(%[p])\n")
CHASE(flush_b_124, "addi 10,%[p],128\n" "dcbf 0,10\n" "ld %[p],124(%[p])\n")
CHASE(flush_a_124, "addi 10,%[p],0\n" "dcbf 0,10\n" "ld %[p],124(%[p])\n")
CHASE(flush_b_112, "addi 10,%[p],128\n" "dcbf 0,10\n" "ld %[p],112(%[p])\n")
CHASE(flush_a_112, "addi 10,%[p],0\n" "dcbf 0,10\n" "ld %[p],112(%[p])\n")
CHASE(touch_ab_124, "addi 10,%[p],2048\n" "dcbt 0,10\n"
                    "addi 11,%[p],2176\n" "dcbt 0,11\n" "ld %[p],124(%[p])\n")
CHASE(touch_aa_124, "addi 10,%[p],2048\n" "dcbt 0,10\n"
                    "addi 11,%[p],2048\n" "dcbt 0,11\n" "ld %[p],124(%[p])\n")
CHASE(touch_bb_124, "addi 10,%[p],2176\n" "dcbt 0,10\n"
                    "addi 11,%[p],2176\n" "dcbt 0,11\n" "ld %[p],124(%[p])\n")
CHASE(touch_ab_112, "addi 10,%[p],2048\n" "dcbt 0,10\n"
                    "addi 11,%[p],2176\n" "dcbt 0,11\n" "ld %[p],112(%[p])\n")

enum kind { COLD, WARM, TOUCH };

struct loop {
    u64 (*run)(u64 *, u64);
    unsigned off;
    enum kind kind;
    enum step step;
};

static const struct loop LOOP[LOOPS] = {
    { ld_192, 192, COLD, STMW_96 },
    { ld_192, 192, COLD, STMW_120 },
    { ld_192, 192, COLD, STMW_128 },
    { flush_b_124, 124, WARM, NONE },
    { flush_a_124, 124, WARM, NONE },
    { flush_b_112, 112, WARM, NONE },
    { flush_a_112, 112, WARM, NONE },
    { touch_ab_124, 124, TOUCH, NONE },
    { touch_aa_124, 124, TOUCH, NONE },
    { touch_bb_124, 124, TOUCH, NONE },
    { touch_ab_112, 112, TOUCH, NONE },
    { ld_112, 112, COLD, LSWX_0 },
    { ld_112, 112, COLD, LBZ },
    { ld_112, 112, COLD, NONE },
};

int main(void)
{
    unsigned i, l, r;
    unsigned next_slot = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    link_ring(ring, RING_NODES, 124);
    link_ring(ring, RING_NODES, 112);
    head124 = EA(ring);
    head112 = EA(ring);

    for (l = 0; l < LOOPS; l++) {
        const struct loop *lp = &LOOP[l];
        unsigned char *slot = lp->kind == TOUCH ? ring : pool[next_slot++];
        u64 n = lp->kind == COLD ? COLD_NODES : WARM_ITER;
        u64 *head = lp->off == 124 ? &head124 : &head112;
        u64 lo = ~0ull, hi = 0;
        if (lp->kind != TOUCH)
            link_ring(slot, lp->kind == COLD ? COLD_NODES : WARM_NODES, lp->off);
        for (r = 0; r < REPS; r++) {
            u64 t;
            u64 start = EA(slot);
            if (lp->kind == COLD)
                prepare(slot, lp->step);
            t = lp->run(lp->kind == TOUCH ? head : &start, n);
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
    put(58, 0x50504C57ull);

    write_frame();
    return 0;
}
