/* PPU probe: whether a sibling's stalls delay a load miss's result.
 * The main thread chases a pointer through an 8 MB ring of 128-byte
 * lines in a shuffled order, so each load misses the L2, with k
 * independent addi after each load. It runs alone and beside a sibling
 * of dependent fdiv or dependent divd.
 *
 *   phase  sibling (an assembly loop: four passes of 8 instructions and a
 *          bdnz, then the stop flag's load and the round count's
 *          increment, the loop's head at a 128-byte line)
 *   0      none
 *   1      dependent fdiv (1.5 / 1.0)
 *   2      dependent divd (0x7fff0000 / 1)
 *
 *   chain  timed loop (4096 iterations, bdnz): one ld of the ring's next
 *          node, then k addi over four targets
 *   0      k = 0
 *   1      k = 8
 *   2      k = 16
 *   3      k = 32
 *
 * The ring's pointer carries over from run to run, so each run visits
 * lines the 4096 loads before it did not.
 *
 * The main thread runs at priority 1001 and the sibling at 2000.
 *
 * Payload: big-endian u64 words. Loop L = 4 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   24 + L         the sibling's rounds while the loop's REPS runs ran
 *   36 + L         the time-base ticks those REPS runs took
 * Phase p:
 *   48 + 2p        the sibling's rounds while the main thread slept
 *   49 + 2p        the time-base ticks of that sleep
 *   54 + p         the sibling's create rc (all ones: none created)
 *   57 + p         its join rc (all ones: not joined)
 *   60 + p         its exit value (expected 0x5157)
 *   63 + p         1 when it reported it runs
 *   66 + p         the sleep's rc (all ones: none slept)
 * Word 70: the sum of every chain's result (keeps each loop live).
 * Word 71: REPS. Word 72: 0x5050534D ("PPSM"): the probe wrote every
 * word.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define MAIN_PRIORITY  1001
#define LOWER_PRIORITY 2000

SYS_PROCESS_PARAM(MAIN_PRIORITY, 0x10000)

#define SYS_PPU_THREAD_EXIT 41
#define SYS_PPU_THREAD_JOIN 44
#define SYS_TIMER_USLEEP    141

#define PHASES  3
#define CHAINS  4
#define WORDS   80
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define ITER    4096u

#define LINE 128
#define RING_BYTES (8u * 1024u * 1024u)

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started;
static volatile unsigned int kind;
static volatile u64 rounds __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));

static unsigned char ring[RING_BYTES] __attribute__((aligned(128)));
static unsigned int order[RING_BYTES / LINE];
static u64 head;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))
#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

#define ADDI4 "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"

struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static struct cg_thread_param sibling_param __attribute__((aligned(8)));

static unsigned long make_thread_param(struct cg_thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    unsigned long tls_reg;
    __asm__ volatile ("mr %0, 13" : "=r"(tls_reg));
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = (unsigned int)tls_reg;
    return (unsigned long)p;
}

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
        u64 *node = (u64 *)(ring + order[i] * LINE);
        *node = EA(ring + order[(i + 1) % n] * LINE);
    }
    return EA(ring + order[0] * LINE);
}

/* The sibling's loop: `body` four times to a bdnz, then stop when the
 * stop flag is set, else count the round. */
#define SIB_LOOP(body)                                             \
    ".p2align 7\n"                                                 \
    "1:\n"                                                         \
    "li 14,4\n"                                                    \
    "mtctr 14\n"                                                   \
    "2:\n" body "bdnz 2b\n"                                        \
    "lwz 0,0(%[stop])\n"                                           \
    "cmpwi 0,0\n"                                                  \
    "bne 3f\n"                                                     \
    "ld 9,0(%[cnt])\n"                                             \
    "addi 9,9,1\n"                                                 \
    "std 9,0(%[cnt])\n"                                            \
    "b 1b\n"                                                       \
    "3:\n"

static void sibling_entry(void *arg)
{
    unsigned k = kind;
    (void)arg;
    started = 1;
    if (k == 1) {
        double a = 1.5, b = 1.0;
        __asm__ volatile (
            SIB_LOOP(R8("fdiv %[a],%[a],%[b]\n"))
            : [a] "+f"(a)
            : [b] "f"(b), [stop] "b"(&stop), [cnt] "b"(&rounds)
            : "r0", "r9", "r14", "ctr", "cr0", "memory");
        sink += (u64)a;
    } else {
        __asm__ volatile (
            "lis 15,0x7fff\n"
            "li 16,1\n"
            SIB_LOOP(R8("divd 15,15,16\n"))
            : : [stop] "b"(&stop), [cnt] "b"(&rounds)
            : "r0", "r9", "r14", "r15", "r16", "ctr", "cr0", "memory");
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n" "ld %[p],0(%[p])\n"
#define LOOP_END   "bdnz 1b\n"
#define CHAIN(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 p = head;                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : [p] "+b"(p) : [n] "r"(n)                             \
            : "r17", "r18", "r19", "r20", "ctr");                  \
        t0 = timebase() - t0;                                      \
        head = p;                                                  \
        sink += p;                                                 \
        return t0;                                                 \
    }

CHAIN(chain_k0, "")
CHAIN(chain_k8, R2(ADDI4))
CHAIN(chain_k16, R4(ADDI4))
CHAIN(chain_k32, R8(ADDI4))

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_k0(ITER);
    case 1: return chain_k8(ITER);
    case 2: return chain_k16(ITER);
    default: return chain_k32(ITER);
    }
}

/* Time chain `c` REPS times as loop `loop`, with the sibling's rounds
 * and the ticks the runs took. */
static void timed(unsigned loop, unsigned c)
{
    u64 lo = ~0ull, hi = 0;
    u64 before = rounds;
    u64 t0 = timebase();
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 t = run_chain(c);
        if (t < lo)
            lo = t;
        if (t > hi)
            hi = t;
    }
    put(36 + loop, timebase() - t0);
    put(24 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    head = build_ring();

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(54 + p, ~0ull);
        put(57 + p, ~0ull);
        put(66 + p, ~0ull);
        if (p == 0) {
            for (c = 0; c < CHAINS; c++)
                timed(CHAINS * p + c, c);
            continue;
        }
        started = 0;
        stop = 0;
        rounds = 0;
        kind = p;
        rc = cgov_ppu_thread_create(
            &tid, make_thread_param(&sibling_param, (const void *)&sibling_entry),
            0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
        put(54 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(63 + p, started);
            before = rounds;
            t0 = timebase();
            put(66 + p, (u64)(s64)syscall1_s32(SYS_TIMER_USLEEP, ALONE_US));
            put(49 + 2 * p, timebase() - t0);
            put(48 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(57 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(60 + p, retval);
    }
    put(70, sink);
    put(71, REPS);
    put(72, 0x5050534Dull);

    write_frame();
    return 0;
}
