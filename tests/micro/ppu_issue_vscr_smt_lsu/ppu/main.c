/* PPU probe: whether one PPU thread's mtvscr stalls the load and store
 * work of the sibling thread beside it. Each phase starts one sibling
 * running one loop that counts its rounds in memory and reads no time
 * base, measures the sibling's rate while the main thread sleeps, then
 * times three chains on the main thread beside it. Phase 0 runs no
 * sibling. Every vector register the sibling reads starts zero, so
 * mtvscr v0 writes a zero VSCR.
 *
 *   phase  sibling round: four passes of 8 instructions and a bdnz, then
 *          a load of the stop flag and an increment of the round count,
 *          the loop's head at a 128-byte line
 *   0      none
 *   1      mtvscr v0
 *   2      independent addi    (four targets)
 *
 *   chain  timed loop (1 << 16 iterations, bdnz), r9 the address of one
 *          128-byte line of the main thread's own
 *   0      8 independent lwz   (four targets, offsets 0 to 12)
 *   1      8 independent lvx   (four targets, the line's first quadword)
 *   2      8 stw               (four sources, offsets 0 to 12)
 *
 * The main thread runs at priority 1001 and the sibling at 2000, so the
 * two hold the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 3 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   20 + L         the sibling's rounds while the loop's REPS runs ran
 *   30 + L         the time-base ticks those REPS runs took
 * Phase p:
 *   40 + 2p        the sibling's rounds while the main thread slept
 *   41 + 2p        the time-base ticks of that sleep
 *   50 + p         the sibling's create rc (all ones: none created)
 *   55 + p         its join rc (all ones: not joined)
 *   60 + p         its exit value (expected 0x5157)
 *   65 + p         1 when it reported it runs
 * Word 70: the line's first doubleword after every chain (keeps each
 * loop live). Word 71: REPS. Word 72: 0x50505753 ("PPWS"): the probe
 * wrote every word.
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
#define CHAINS  3
#define WORDS   80
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define ITER    (1u << 16)
#define DONE    0x50505753ull

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static volatile unsigned int started;
static volatile unsigned int kind;
/* Each on its own line: the sibling stores `rounds` every round and
 * loads `stop`; the main thread reads `rounds` only between loops. */
static volatile u64 rounds __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));
/* The line the main thread's chains load and store. */
static volatile u64 line[16] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x

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

#define ADDI4                                                   \
    "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"

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
        : "r0", "r9", "r14", "r17", "r18", "r19", "r20", "ctr",    \
          "cr0", "memory", "v0", "v20", "v21")

static void sibling_entry(void *arg)
{
    unsigned k = kind;
    (void)arg;
    started = 1;
    switch (k) {
    case 1:
        SIB(R2(R2(R2("mtvscr 0\n"))));
        break;
    default:
        SIB(R2(ADDI4));
        break;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

#define LOOP_BEGIN "mr 9,%[p]\n" "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define LSLOOP(name, body)                                         \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n), [p] "b"(line)                          \
            : "r9", "r17", "r18", "r19", "r20", "ctr", "memory",   \
              "v1", "v2", "v3", "v4");                             \
        return timebase() - t0;                                    \
    }

#define LWZ4                                                       \
    "lwz 17,0(9)\n" "lwz 18,4(9)\n" "lwz 19,8(9)\n" "lwz 20,12(9)\n"
#define LVX4                                                       \
    "lvx 1,0,9\n" "lvx 2,0,9\n" "lvx 3,0,9\n" "lvx 4,0,9\n"
#define STW4                                                       \
    "stw 17,0(9)\n" "stw 18,4(9)\n" "stw 19,8(9)\n" "stw 20,12(9)\n"

LSLOOP(chain_lwz, R2(LWZ4))
LSLOOP(chain_lvx, R2(LVX4))
LSLOOP(chain_stw, R2(STW4))

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_lwz(ITER);
    case 1: return chain_lvx(ITER);
    default: return chain_stw(ITER);
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
    put(30 + loop, timebase() - t0);
    put(20 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 16; i++)
        line[i] = 0;

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(50 + p, ~0ull);
        put(55 + p, ~0ull);
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
        put(50 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(65 + p, started);
            before = rounds;
            t0 = timebase();
            syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(41 + 2 * p, timebase() - t0);
            put(40 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(55 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(60 + p, retval);
    }
    put(70, line[0]);
    put(71, REPS);
    put(72, DONE);

    write_frame();
    return 0;
}
