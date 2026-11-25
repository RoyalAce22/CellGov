/* PPU probe: whether an mtvscr that holds an FP or VMX load in the VSU
 * issue queue also delays the load's cache miss. Each chase loop follows
 * a pointer through FP or vector registers: the load reads the node's
 * first doubleword, a store puts it in a scratch doubleword, and an ld
 * takes it back into r9 as the next node's address. So each iteration
 * waits for the load before it.
 *
 * The ring is 8 MB of 128-byte lines linked in an LCG-shuffled order, so
 * each load of a ring node misses the L2. Its head carries over from run
 * to run. The node is one 128-byte line whose first doubleword holds its
 * own address, so each load of it hits the L1.
 *
 * Every vector register the loops read is zero, so mtvscr v0 writes a
 * zero VSCR.
 *
 *   loop  body (r9 = node, r8 = scratch; bdnz)           nodes   iterations
 *   0     ld r9,0(r9)                                     ring    2^12
 *   1     lfd f1,0(r9); stfd f1,0(r8); ld r9,0(r8)        ring    2^12
 *   2     mtvscr v0; then loop 1's body                   ring    2^12
 *   3     lvx v1,0,r9; stvx v1,0,r8; ld r9,0(r8)          ring    2^12
 *   4     mtvscr v0; then loop 3's body                   ring    2^12
 *   5     loop 1's body                                   node    2^14
 *   6     loop 2's body                                   node    2^14
 *   7     loop 3's body                                   node    2^14
 *   8     loop 4's body                                   node    2^14
 *
 * Each loop runs REPS times between two time-base reads a run.
 *
 * Payload: big-endian u64 words.
 *   2i, 2i + 1  loop i's smallest and largest time-base delta
 *   18          1 when every ring run ended on a ring line
 *   19          1 when every node run ended on the node
 *   20          REPS
 *   21          log2 of the ring loops' iterations
 *   22          log2 of the node loops' iterations
 *   23          0x50505658 ("PPVX"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     24
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define RING_LOG2 12
#define NODE_LOG2 14

#define LINE 128
#define RING_BYTES (8u * 1024u * 1024u)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static unsigned char ring[RING_BYTES] __attribute__((aligned(128)));
static unsigned int order[RING_BYTES / LINE];
static u64 node[LINE / 8] __attribute__((aligned(128)));
static u64 scratch[LINE / 8] __attribute__((aligned(128)));
static u64 head;
static unsigned ring_ok = 1;
static unsigned node_ok = 1;

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

/* Link the ring into one cycle of one doubleword per line, in an
 * LCG-shuffled order so a stride prefetch cannot follow it. Returns the
 * address of the first node. */
static u64 build_ring(void)
{
    unsigned n = RING_BYTES / LINE;
    unsigned i;
    u64 seed = 0x2545F4914F6CDD1Dull;
    for (i = 0; i < n; i++)
        order[i] = i;
    for (i = n - 1; i > 0; i--) {
        unsigned j, t;
        seed = seed * 6364136223846793005ull + 1442695040888963407ull;
        j = (unsigned)((seed >> 33) % (i + 1));
        t = order[i];
        order[i] = order[j];
        order[j] = t;
    }
    for (i = 0; i < n; i++) {
        u64 *at = (u64 *)(ring + order[i] * LINE);
        *at = EA(ring + order[(i + 1) % n] * LINE);
    }
    return EA(ring + order[0] * LINE);
}

#define LOOP_BEGIN "vxor 0,0,0\n" "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define MTVSCR "mtvscr 0\n"
#define FP_CHASE  "lfd 1,0(9)\n" "stfd 1,0(8)\n" "ld 9,0(8)\n"
#define VMX_CHASE "lvx 1,0,9\n" "stvx 1,0,8\n" "ld 9,0(8)\n"

/* One chase loop of n iterations from *p, which it moves to the last
 * node; returns the time-base delta. */
#define CHASE(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 *p, u64 n)       \
    {                                                              \
        register u64 r9 __asm__("r9") = *p;                        \
        register u64 r8 __asm__("r8") = EA(scratch);               \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(r9)                                             \
            : [n] "r"(n), "r"(r8)                                  \
            : "ctr", "memory", "v0", "v1", "fr1");                 \
        t0 = timebase() - t0;                                      \
        *p = r9;                                                   \
        return t0;                                                 \
    }

CHASE(chase_ld, "ld 9,0(9)\n")
CHASE(chase_fp, FP_CHASE)
CHASE(chase_vscr_fp, MTVSCR FP_CHASE)
CHASE(chase_vmx, VMX_CHASE)
CHASE(chase_vscr_vmx, MTVSCR VMX_CHASE)

typedef u64 (*chase_fn)(u64 *, u64);

static const chase_fn RING_LOOPS[5] = {
    chase_ld, chase_fp, chase_vscr_fp, chase_vmx, chase_vscr_vmx,
};
static const chase_fn NODE_LOOPS[4] = {
    chase_fp, chase_vscr_fp, chase_vmx, chase_vscr_vmx,
};

static int on_ring_line(u64 p)
{
    return p >= EA(ring) && p < EA(ring) + RING_BYTES && (p & (LINE - 1)) == 0;
}

/* Run `f` REPS times as loop `loop`, from *p; keep the smallest and the
 * largest delta. */
static void timed(unsigned loop, chase_fn f, u64 *p, u64 n, int on_ring)
{
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 t = f(p, n);
        if (t < lo)
            lo = t;
        if (t > hi)
            hi = t;
        if (on_ring && !on_ring_line(*p))
            ring_ok = 0;
        if (!on_ring && *p != EA(node))
            node_ok = 0;
    }
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i;
    u64 at;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < LINE / 8; i++) {
        node[i] = 0;
        scratch[i] = 0;
    }
    node[0] = EA(node);
    head = build_ring();

    for (i = 0; i < 5; i++)
        timed(i, RING_LOOPS[i], &head, 1ull << RING_LOG2, 1);
    for (i = 0; i < 4; i++) {
        at = EA(node);
        timed(5 + i, NODE_LOOPS[i], &at, 1ull << NODE_LOG2, 0);
    }

    put(18, ring_ok);
    put(19, node_ok);
    put(20, REPS);
    put(21, RING_LOG2);
    put(22, NODE_LOG2);
    put(23, 0x50505658ull);

    write_frame();
    return 0;
}
