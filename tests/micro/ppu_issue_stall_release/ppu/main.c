/* PPU probe: whether a sibling's stalls delay a store's release from the
 * store queue to a load that reloads its bytes, and a divide's release of
 * the FXU. The main thread times seven loops alone and beside a sibling
 * of dependent fdiv.
 *
 *   phase  sibling (an assembly loop: four passes of 8 fdiv and a bdnz,
 *          then the stop flag's load and the round count's increment,
 *          the loop's head at a 128-byte line)
 *   0      none
 *   1      dependent fdiv (1.5 / 1.0)
 *
 *   chain  timed loop (4096 iterations, bdnz), its body four times; r7
 *          and r8 point at two lines, r15 = 0x7fff0000, r16 = 1
 *   0      std 9,0(7); ld 9,0(7)                (reload the stored bytes)
 *   1      std 9,0(7); ld 9,0(8)                (load another line)
 *   2      std 9,0(7); addi 17..20; ld 9,0(7)
 *   3      std 9,0(7); addi 17..20; ld 9,0(8)
 *   4      divd 17+i,15,16                      (independent divides)
 *   5      divd 17+i,15,16; ld 21..24,0..24(8)
 *   6      addi 17+i,17+i,1; ld 21..24,0..24(8)
 *
 * The main thread runs at priority 1001 and the sibling at 2000.
 *
 * In phase 1 a run counts only when the sibling kept at least half the
 * rate it ran at while the main thread slept: the kernel can take the
 * sibling's hardware thread for a while, and a run then times no
 * sibling. Each loop runs until REPS runs count, or MAX_RUNS runs.
 *
 * Payload: big-endian u64 words. Loop L = 7 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  the runs that count
 *   28 + L         the sibling's rounds while those runs ran
 *   42 + L         the time-base ticks those runs took
 *   56 + L         the runs that count
 *   70 + L         the runs made
 * The sibling of phase 1:
 *   84             its rounds while the main thread slept
 *   85             the time-base ticks of that sleep
 *   86             its create rc (all ones: none created)
 *   87             its join rc (all ones: not joined)
 *   88             its exit value (expected 0x5157)
 *   89             1 when it reported it runs
 *   90             the sleep's rc (all ones: none slept)
 * Word 91: the sum of every chain's result (keeps each loop live).
 * Word 92: REPS. Word 93: 0x50505352 ("PPSR"): the probe wrote every
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

#define PHASES  2
#define CHAINS  7
#define WORDS   94
#define PAYLOAD (WORDS * 8)
#define REPS    15
#define MAX_RUNS (8 * REPS)
#define ITER    4096u

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started;
static volatile u64 rounds __attribute__((aligned(128)));
/* The sibling's rounds and the ticks of the main thread's sleep; zero
 * rounds while no sibling runs. */
static u64 alone_rounds, alone_ticks;
static volatile unsigned int stop __attribute__((aligned(128)));

static volatile u64 line_a[16] __attribute__((aligned(128)));
static volatile u64 line_b[16] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

#define ADDI4 "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"
#define LD4   "ld 21,0(8)\n" "ld 22,8(8)\n" "ld 23,16(8)\n" "ld 24,24(8)\n"

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
    double a = 1.5, b = 1.0;
    (void)arg;
    started = 1;
    __asm__ volatile (
        SIB_LOOP(R8("fdiv %[a],%[a],%[b]\n"))
        : [a] "+f"(a)
        : [b] "f"(b), [stop] "b"(&stop), [cnt] "b"(&rounds)
        : "r0", "r9", "r14", "ctr", "cr0", "memory");
    sink += (u64)a;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

/* Each chain sets its registers, then runs its loop `n` times. */
#define LOOP_BEGIN                                                 \
    "mr 7,%[a]\n" "mr 8,%[b]\n" "li 9,5\n"                         \
    "lis 15,0x7fff\n" "li 16,1\n"                                  \
    "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END "bdnz 1b\n" "add %[r],9,17\n" "add %[r],%[r],21\n"
#define CHAIN(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 r;                                                     \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : [r] "=&r"(r)                                         \
            : [n] "r"(n), [a] "r"(line_a), [b] "r"(line_b)         \
            : "r7", "r8", "r9", "r15", "r16", "r17", "r18", "r19", \
              "r20", "r21", "r22", "r23", "r24", "ctr", "memory"); \
        t0 = timebase() - t0;                                      \
        sink += r;                                                 \
        return t0;                                                 \
    }

CHAIN(chain_reload, R4("std 9,0(7)\n" "ld 9,0(7)\n"))
CHAIN(chain_other, R4("std 9,0(7)\n" "ld 9,0(8)\n"))
CHAIN(chain_reload_k4, R4("std 9,0(7)\n" ADDI4 "ld 9,0(7)\n"))
CHAIN(chain_other_k4, R4("std 9,0(7)\n" ADDI4 "ld 9,0(8)\n"))
CHAIN(chain_divd, "divd 17,15,16\n" "divd 18,15,16\n"
                  "divd 19,15,16\n" "divd 20,15,16\n")
CHAIN(chain_divd_ld, "divd 17,15,16\n" LD4 "divd 18,15,16\n" LD4
                     "divd 19,15,16\n" LD4 "divd 20,15,16\n" LD4)
CHAIN(chain_addi_ld, "addi 17,17,1\n" LD4 "addi 18,18,1\n" LD4
                     "addi 19,19,1\n" LD4 "addi 20,20,1\n" LD4)

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_reload(ITER);
    case 1: return chain_other(ITER);
    case 2: return chain_reload_k4(ITER);
    case 3: return chain_other_k4(ITER);
    case 4: return chain_divd(ITER);
    case 5: return chain_divd_ld(ITER);
    default: return chain_addi_ld(ITER);
    }
}

/* Time chain `c` as loop `loop` until REPS runs count, with the
 * sibling's rounds and the ticks of the runs that count. With a sibling,
 * a run counts when the sibling kept at least half its sleep-time rate. */
static void timed(unsigned loop, unsigned c)
{
    u64 lo = ~0ull, hi = 0, ticks = 0, sib = 0;
    unsigned runs = 0, counted = 0;
    while (counted < REPS && runs < MAX_RUNS) {
        u64 before = rounds;
        u64 t0 = timebase();
        u64 t, n;
        run_chain(c);
        t = timebase() - t0;
        n = rounds - before;
        runs++;
        if (alone_rounds != 0 && 2 * n * alone_ticks < alone_rounds * t)
            continue;
        counted++;
        ticks += t;
        sib += n;
        if (t < lo)
            lo = t;
        if (t > hi)
            hi = t;
    }
    put(42 + loop, ticks);
    put(28 + loop, sib);
    put(56 + loop, counted);
    put(70 + loop, runs);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, c;
    unsigned long long tid = 0, retval = 0;
    s32 rc;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 16; i++) {
        line_a[i] = i;
        line_b[i] = i;
    }

    for (c = 0; c < CHAINS; c++)
        timed(c, c);

    put(86, ~0ull);
    put(87, ~0ull);
    put(90, ~0ull);
    started = 0;
    stop = 0;
    rounds = 0;
    rc = cgov_ppu_thread_create(
        &tid, make_thread_param(&sibling_param, (const void *)&sibling_entry),
        0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
    put(86, (u64)(s64)rc);
    if (rc == 0) {
        u64 t0 = timebase();
        u64 before;
        while (!started && timebase() - t0 < START_WAIT_TB) {
        }
        put(89, started);
        before = rounds;
        t0 = timebase();
        put(90, (u64)(s64)syscall1_s32(SYS_TIMER_USLEEP, ALONE_US));
        alone_ticks = timebase() - t0;
        alone_rounds = rounds - before;
        put(85, alone_ticks);
        put(84, alone_rounds);
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS + c, c);
        stop = 1;
        put(87, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(88, retval);
    }
    put(91, sink);
    put(92, REPS);
    put(93, 0x50505352ull);

    write_frame();
    return 0;
}
