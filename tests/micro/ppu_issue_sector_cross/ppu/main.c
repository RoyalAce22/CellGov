/* PPU probe: what an integer load costs when its bytes cross from one
 * 32-byte L1 sector into the next inside one 128-byte line, by width and
 * by where in the sector it starts. One thread; no LV2 call before the
 * result frame.
 *
 * Each loop gets its own 8 KB slot of `pool`: a ring of 64 nodes of one
 * line each. Each node holds a link to the next node at the loop's
 * offset, in the node's third sector (bytes 64 to 95) or reaching into
 * its fourth:
 *   - an ldx loop stores the next node's address as a big-endian
 *     doubleword, and runs `ldx p,p,o`;
 *   - an lwzx loop stores it as a big-endian word, and runs `lwzx p,p,o`;
 *   - an lhzx loop stores the next node's offset from the slot as a
 *     big-endian halfword, and runs `lhzx t,p,o; add p,b,t`.
 * Each loop then runs bdnz, so each load waits for the one before it.
 * Each loop chases its ring 1 << 14 times a run; earlier runs leave the
 * ring in the L1.
 *
 *   loop  load   offset  sector offset  bytes
 *   0     ldx    80      16             one sector
 *   1     ldx    88      24             one sector, ends at its end
 *   2     ldx    92      28             two sectors, each word in one
 *   3     ldx    89      25             two sectors, a word across both
 *   4     ldx    90      26             two sectors, a word across both
 *   5     ldx    93      29             two sectors, a word across both
 *   6     ldx    95      31             two sectors, a word across both
 *   7     lwzx   80      16             one sector
 *   8     lwzx   90      26             one sector, misaligned
 *   9     lwzx   94      30             two sectors, each halfword in one
 *   10    lwzx   93      29             two sectors, a halfword across both
 *   11    lwzx   95      31             two sectors, a halfword across both
 *   12    lhzx   80      16             one sector
 *   13    lhzx   94      30             one sector, ends at its end
 *   14    lhzx   95      31             two sectors
 *   15    lhzx   93      29             one sector, misaligned
 *
 * Payload: big-endian u64 words. For loop L:
 *   2L, 2L + 1   the smallest and the largest time-base delta of REPS runs
 *   32 + L       the effective address of the loop's slot
 *   48 + L       the loop's iterations a run
 * Word 64: the sum of every chase's final pointer (keeps each loop live).
 * Word 65: REPS. Word 66: 0x50505358 ("PPSX"): the probe wrote every word.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS   16
#define WORDS   67
#define PAYLOAD (WORDS * 8)
#define REPS    8

#define NODE    128
#define NODES   64
#define SLOT    (NODES * NODE)
#define ITER    (1u << 14)

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

/* Link the ring of `slot`: each node holds, in `bytes` bytes at `at`, the
 * next node's address, or for a halfword its offset from the slot. */
static void link_ring(unsigned char *slot, unsigned at, unsigned bytes)
{
    unsigned i;
    for (i = 0; i < NODES; i++) {
        unsigned next = ((i + 1) % NODES) * NODE;
        u64 link = bytes == 2 ? (u64)next : EA(slot + next);
        put_be(slot + i * NODE + at, link, bytes);
    }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

static u64 __attribute__((noinline)) chase_ldx(u64 p, u64 o, u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN "ldx %[p],%[p],%[o]\n" LOOP_END
        : [p] "+b"(p) : [o] "r"(o), [n] "r"(n) : "ctr", "memory");
    t0 = timebase() - t0;
    sink += p;
    return t0;
}

static u64 __attribute__((noinline)) chase_lwzx(u64 p, u64 o, u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN "lwzx %[p],%[p],%[o]\n" LOOP_END
        : [p] "+b"(p) : [o] "r"(o), [n] "r"(n) : "ctr", "memory");
    t0 = timebase() - t0;
    sink += p;
    return t0;
}

static u64 __attribute__((noinline)) chase_lhzx(u64 p, u64 o, u64 n)
{
    u64 b = p;
    u64 t;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN "lhzx %[t],%[p],%[o]\n" "add %[p],%[b],%[t]\n" LOOP_END
        : [p] "+b"(p), [t] "=&r"(t)
        : [b] "r"(b), [o] "r"(o), [n] "r"(n) : "ctr", "memory");
    t0 = timebase() - t0;
    sink += p;
    return t0;
}

struct loop {
    u64 (*run)(u64, u64, u64);
    unsigned off;
    unsigned bytes;
};

static const struct loop LOOP[LOOPS] = {
    { chase_ldx, 80, 8 },
    { chase_ldx, 88, 8 },
    { chase_ldx, 92, 8 },
    { chase_ldx, 89, 8 },
    { chase_ldx, 90, 8 },
    { chase_ldx, 93, 8 },
    { chase_ldx, 95, 8 },
    { chase_lwzx, 80, 4 },
    { chase_lwzx, 90, 4 },
    { chase_lwzx, 94, 4 },
    { chase_lwzx, 93, 4 },
    { chase_lwzx, 95, 4 },
    { chase_lhzx, 80, 2 },
    { chase_lhzx, 94, 2 },
    { chase_lhzx, 95, 2 },
    { chase_lhzx, 93, 2 },
};

int main(void)
{
    unsigned i, l, r;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (l = 0; l < LOOPS; l++) {
        const struct loop *lp = &LOOP[l];
        unsigned char *slot = pool[l];
        u64 lo = ~0ull, hi = 0;
        link_ring(slot, lp->off, lp->bytes);
        for (r = 0; r < REPS; r++) {
            u64 t = lp->run(EA(slot), lp->off, ITER);
            if (t < lo)
                lo = t;
            if (t > hi)
                hi = t;
        }
        put(2 * l, lo);
        put(2 * l + 1, hi);
        put(32 + l, EA(slot));
        put(48 + l, ITER);
    }
    put(64, sink);
    put(65, REPS);
    put(66, 0x50505358ull);

    write_frame();
    return 0;
}
