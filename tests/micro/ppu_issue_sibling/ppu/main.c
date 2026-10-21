/* PPU probe: what each timed chain costs the sibling PPU thread beside
 * it. Each phase starts one sibling running one instruction class in a
 * loop that counts its rounds in memory and reads no time base,
 * measures the sibling's rate while the main thread sleeps, then times
 * five chains on the main thread beside it. Phase 0 runs no sibling.
 *
 *   phase  sibling round: four passes of 8 instructions and a bdnz, then
 *          a load of the stop flag and an increment of the round count,
 *          the loop's head at a 128-byte line
 *   0      none
 *   1      independent addi    (four targets)
 *   2      dependent divd      (0x7fff0000 / 1)
 *   3      dependent mulld     (x * 1)
 *   4      dependent fdiv      (1.5 / 1.0)
 *   5      dependent fadd      (x + 0)
 *   6      microcoded lha      (four targets, one L1 line)
 *
 *   chain  timed loop (1 << 16 iterations, bdnz)
 *   0      8 dependent add
 *   1      8 dependent mulli
 *   2      16 independent addi (four targets)
 *   3      8 dependent fadd
 *   4      8 dependent ld (a pointer to itself, an L1 hit)
 *
 * The main thread runs at priority 1001 and the sibling at 2000, so the
 * two hold the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 5 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   70 + L         the sibling's rounds while the loop's REPS runs ran
 *   105 + L        the time-base ticks those REPS runs took
 * Phase p:
 *   140 + 2p       the sibling's rounds while the main thread slept
 *   141 + 2p       the time-base ticks of that sleep
 *   154 + p        the sibling's create rc (all ones: none created)
 *   161 + p        its join rc (all ones: not joined)
 *   168 + p        its exit value (expected 0x5157)
 *   175 + p        1 when it reported it runs
 * Word 182: the sum of every chain's result (keeps each loop live).
 * Word 183: REPS. Word 184: 0x50505342 ("PPSB"): the probe wrote every
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

#define PHASES  7
#define CHAINS  5
#define WORDS   192
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define ITER    (1u << 16)

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started;
static volatile unsigned int kind;
/* Each on its own line: the sibling stores `rounds` every round and
 * loads `stop`; the main thread reads `rounds` only between loops. */
static volatile u64 rounds __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));

static unsigned short halves[64] __attribute__((aligned(128)));
static u64 self_ptr __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

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

#define SIB_GPR(body)                                              \
    __asm__ volatile (                                             \
        "lis 15,0x7fff\n"                                          \
        "li 16,1\n"                                                \
        SIB_LOOP(body)                                             \
        : : [stop] "b"(&stop), [cnt] "b"(&rounds)                  \
        : "r0", "r9", "r14", "r15", "r16", "r17", "r18", "r19",    \
          "r20", "ctr", "cr0", "memory")

#define SIB_FPR(body, a0, b0)                                      \
    do {                                                           \
        double a_ = (a0), b_ = (b0);                               \
        __asm__ volatile (                                         \
            SIB_LOOP(body)                                         \
            : [a] "+f"(a_)                                         \
            : [b] "f"(b_), [stop] "b"(&stop), [cnt] "b"(&rounds)   \
            : "r0", "r9", "r14", "ctr", "cr0", "memory");          \
        sink += (u64)a_;                                           \
    } while (0)

static void sibling_entry(void *arg)
{
    unsigned k = kind;
    (void)arg;
    started = 1;
    switch (k) {
    case 1:
        SIB_GPR(R2("addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"));
        break;
    case 2:
        SIB_GPR(R8("divd 15,15,16\n"));
        break;
    case 3:
        SIB_GPR(R8("mulld 15,15,16\n"));
        break;
    case 4:
        SIB_FPR(R8("fdiv %[a],%[a],%[b]\n"), 1.5, 1.0);
        break;
    case 5:
        SIB_FPR(R8("fadd %[a],%[a],%[b]\n"), 1.0, 0.0);
        break;
    default:
        __asm__ volatile (
            SIB_LOOP(R2("lha 17,0(%[p])\n" "lha 18,2(%[p])\n"
                        "lha 19,4(%[p])\n" "lha 20,6(%[p])\n"))
            : : [p] "b"(halves), [stop] "b"(&stop), [cnt] "b"(&rounds)
            : "r0", "r9", "r14", "r17", "r18", "r19", "r20", "ctr", "cr0",
              "memory");
        break;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

static u64 __attribute__((noinline)) chain_add(u64 n)
{
    u64 a = 1;
    u64 t0 = timebase();
    __asm__ volatile (
        "li 15,3\n"
        LOOP_BEGIN R8("add %[a],%[a],15\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_mulli(u64 n)
{
    u64 a = 7;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("mulli %[a],%[a],1\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_addi(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        R4("addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n")
        LOOP_END
        : : [n] "r"(n) : "r17", "r18", "r19", "r20", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_fadd(u64 n)
{
    double a = 1.0, z = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("fadd %[a],%[a],%[z]\n") LOOP_END
        : [a] "+f"(a) : [z] "f"(z), [n] "r"(n) : "ctr");
    sink += (u64)a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_ld(u64 n)
{
    u64 p = (u64)(unsigned long)&self_ptr;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("ld %[p],0(%[p])\n") LOOP_END
        : [p] "+b"(p) : [n] "r"(n) : "ctr");
    sink += p;
    return timebase() - t0;
}

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_add(ITER);
    case 1: return chain_mulli(ITER);
    case 2: return chain_addi(ITER);
    case 3: return chain_fadd(ITER);
    default: return chain_ld(ITER);
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
    put(105 + loop, timebase() - t0);
    put(70 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    self_ptr = (u64)(unsigned long)&self_ptr;
    for (i = 0; i < 64; i++)
        halves[i] = (unsigned short)i;

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(154 + p, ~0ull);
        put(161 + p, ~0ull);
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
        put(154 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(175 + p, started);
            before = rounds;
            t0 = timebase();
            syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(141 + 2 * p, timebase() - t0);
            put(140 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(161 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(168 + p, retval);
    }
    put(182, sink);
    put(183, REPS);
    put(184, 0x50505342ull);

    write_frame();
    return 0;
}
