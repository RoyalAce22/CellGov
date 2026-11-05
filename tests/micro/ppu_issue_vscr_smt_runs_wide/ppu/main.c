/* PPU probe: what a chain of independent vaddubm costs an iteration
 * beside a sibling thread whose vector work waits behind its own mtvscr,
 * by the chain's length, with every run recorded. Each phase starts one
 * sibling running one loop that counts its rounds in memory and reads no
 * time base, measures the sibling's rate while the main thread sleeps,
 * then times ten chains on the main thread beside it, REPS runs each.
 * Phase 0 runs no sibling. Every vector register either thread reads
 * starts zero, so mtvscr v0 writes a zero VSCR.
 *
 *   phase  sibling round: four passes of its body and a bdnz, then a
 *          load of the stop flag and an increment of the round count,
 *          the loop's head at a 128-byte line
 *   0      none
 *   1      mtvscr v0 and 3 independent vaddubm
 *   2      mtvscr v0 and 15 independent vaddubm (seven targets)
 *
 *   chain  timed loop (1 << 16 iterations, bdnz)
 *   0      1 vaddubm
 *   1      2 independent vaddubm
 *   2      3 independent vaddubm
 *   3      4 independent vaddubm
 *   4      6 independent vaddubm
 *   5      8 independent vaddubm
 *   6      12 independent vaddubm (eight targets)
 *   7      16 independent vaddubm (eight targets)
 *   8      2 independent vaddubm and 8 li (four targets)
 *   9      8 li (four targets)
 *
 * The main thread runs at priority 1001 and the sibling at 2000, so the
 * two hold the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 10 * phase + chain, run r:
 *   2 * (9L + r)       the run's time-base delta
 *   2 * (9L + r) + 1   the sibling's rounds while the run ran
 * Phase p, at 540 + 8p:
 *   +0  the sibling's rounds while the main thread slept
 *   +1  the time-base ticks of that sleep
 *   +2  the sibling's create rc (all ones: none created)
 *   +3  its join rc (all ones: not joined)
 *   +4  its exit value (expected 0x5157)
 *   +5  1 when it reported it runs
 *   +6  the sleep's rc
 * Word 564: REPS. Word 565: log2 of the iterations. Word 566:
 * 0x50505657 ("PPVW"): the probe wrote every word.
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

#define PHASES    3
#define CHAINS    10
#define REPS      9
#define META      540
#define WORDS     567
#define PAYLOAD   (WORDS * 8)
#define ITER_LOG2 16
#define DONE      0x50505657ull

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static volatile unsigned int started;
static volatile unsigned int kind;
/* Each on its own line: the sibling stores `rounds` every round and
 * loads `stop`; the main thread reads `rounds` only between runs. */
static volatile u64 rounds __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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

#define VZERO "vxor 0,0,0\n" "vxor 20,20,20\n" "vxor 21,21,21\n"

#define VADD1 "vaddubm 1,20,21\n"
#define VADD2 VADD1 "vaddubm 2,20,21\n"
#define VADD3 VADD2 "vaddubm 3,20,21\n"
#define VADD4 VADD3 "vaddubm 4,20,21\n"
#define VADD6 VADD4 "vaddubm 5,20,21\n" "vaddubm 6,20,21\n"
#define VADD7 VADD6 "vaddubm 7,20,21\n"
#define VADD8 VADD7 "vaddubm 8,20,21\n"
#define LI4   "li 17,1\n" "li 18,1\n" "li 19,1\n" "li 20,1\n"
#define LI8   LI4 LI4

/* The sibling's loop: `body` four times to a bdnz, then stop when the
 * stop flag is set, else count the round. The head sits at a 128-byte
 * line. */
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

#define SIB(body)                                                  \
    __asm__ volatile (                                             \
        VZERO SIB_LOOP(body)                                       \
        : : [stop] "b"(&stop), [cnt] "b"(&rounds)                  \
        : "r0", "r9", "r14", "ctr", "cr0", "memory", "v0", "v1",   \
          "v2", "v3", "v4", "v5", "v6", "v7", "v20", "v21")

static void sibling_entry(void *arg)
{
    unsigned k = kind;
    (void)arg;
    started = 1;
    if (k == 1)
        SIB("mtvscr 0\n" VADD3);
    else
        SIB("mtvscr 0\n" VADD7 VADD7 VADD1);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define VLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            VZERO LOOP_BEGIN body LOOP_END                         \
            : : [n] "r"(n)                                         \
            : "r17", "r18", "r19", "r20", "ctr", "v0", "v1", "v2", \
              "v3", "v4", "v5", "v6", "v7", "v8", "v20", "v21");   \
        return timebase() - t0;                                    \
    }

VLOOP(chain_vadd1, VADD1)
VLOOP(chain_vadd2, VADD2)
VLOOP(chain_vadd3, VADD3)
VLOOP(chain_vadd4, VADD4)
VLOOP(chain_vadd6, VADD6)
VLOOP(chain_vadd8, VADD8)
VLOOP(chain_vadd12, VADD8 VADD4)
VLOOP(chain_vadd16, VADD8 VADD8)
VLOOP(chain_vadd2_li8, VADD2 LI8)
VLOOP(chain_li8, LI8)

static u64 run_chain(unsigned c)
{
    const u64 n = 1ull << ITER_LOG2;
    switch (c) {
    case 0: return chain_vadd1(n);
    case 1: return chain_vadd2(n);
    case 2: return chain_vadd3(n);
    case 3: return chain_vadd4(n);
    case 4: return chain_vadd6(n);
    case 5: return chain_vadd8(n);
    case 6: return chain_vadd12(n);
    case 7: return chain_vadd16(n);
    case 8: return chain_vadd2_li8(n);
    default: return chain_li8(n);
    }
}

/* Time chain `c` REPS times as loop `loop`: each run's ticks and the
 * sibling's rounds over it. */
static void timed(unsigned loop, unsigned c)
{
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 before = rounds;
        u64 t = run_chain(c);
        put(2 * (REPS * loop + r), t);
        put(2 * (REPS * loop + r) + 1, rounds - before);
    }
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        unsigned m = META + 8 * p;
        s32 rc;
        put(m + 2, ~0ull);
        put(m + 3, ~0ull);
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
        put(m + 2, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(m + 5, started);
            before = rounds;
            t0 = timebase();
            put(m + 6, (u64)(s64)syscall1_s32(SYS_TIMER_USLEEP, ALONE_US));
            put(m + 1, timebase() - t0);
            put(m + 0, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(m + 3, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(m + 4, retval);
    }
    put(564, REPS);
    put(565, ITER_LOG2);
    put(566, DONE);

    write_frame();
    return 0;
}
