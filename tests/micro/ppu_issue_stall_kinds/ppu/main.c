/* PPU probe: how much of a sibling's stall each kind of instruction
 * waits. The main thread times twenty loops alone and beside a sibling of
 * dependent fdiv: five families, each with K = 0, 2, 4 or 8 addi.
 *
 *   phase  sibling (an assembly loop: four passes of 8 fdiv and a bdnz,
 *          then the stop flag's load and the round count's increment,
 *          the loop's head at a 128-byte line)
 *   0      none
 *   1      dependent fdiv (1.5 / 1.0)
 *
 *   chain  timed loop (ITER iterations, bdnz); chain 4f + k has K = 0, 2,
 *          4, 8 addi for k = 0..3. r8 points at one line. The addi are
 *          addi 17..24,17..24,1 in turn.
 *   0..3   K addi
 *   4..7   ld 9,0(8); K addi                    (a hit)
 *   8..11  std 9,0(8); K addi                   (a store)
 *   12..15 std 9,0(8); K addi; ld 9,0(8)        (a store, then its reload)
 *   16..19 mtvscr 0; K addi; vaddubm 1,2,3
 *
 * The main thread runs at priority 1001 and the sibling at 2000. Every
 * vector register the loops read starts zero.
 *
 * In phase 1 a run counts only when the sibling kept at least half the
 * rate it ran at while the main thread slept. Each loop runs until REPS
 * runs count, or MAX_RUNS runs.
 *
 * Payload: big-endian u64 words. Loop L = 20 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  the runs that count
 *   80 + L         the sibling's rounds while those runs ran
 *   120 + L        the time-base ticks those runs took
 *   160 + L        the runs that count
 *   200 + L        the runs made
 * The sibling of phase 1:
 *   240            its rounds while the main thread slept
 *   241            the time-base ticks of that sleep
 *   242            its create rc (all ones: none created)
 *   243            its join rc (all ones: not joined)
 *   244            its exit value (expected 0x5157)
 *   245            1 when it reported it runs
 *   246            the sleep's rc (all ones: none slept)
 * Word 247: the sum of every chain's result (keeps each loop live).
 * Word 248: REPS. Word 249: ITER. Word 250: 0x5050534b ("PPSK"): the
 * probe wrote every word.
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

#define PHASES   2
#define CHAINS   20
#define WORDS    251
#define PAYLOAD  (WORDS * 8)
#define REPS     15
#define MAX_RUNS (8 * REPS)
#define ITER     2048u

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

static volatile u64 line[16] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

#define ADDI0 ""
#define ADDI2 "addi 17,17,1\n" "addi 18,18,1\n"
#define ADDI4 ADDI2 "addi 19,19,1\n" "addi 20,20,1\n"
#define ADDI8 ADDI4 "addi 21,21,1\n" "addi 22,22,1\n" "addi 23,23,1\n" "addi 24,24,1\n"

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
    "mr 8,%[b]\n" "li 9,5\n"                                       \
    "vxor 0,0,0\n" "vxor 2,2,2\n" "vxor 3,3,3\n"                   \
    "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END "bdnz 1b\n" "add %[r],9,17\n" "add %[r],%[r],24\n"
#define CHAIN(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 r;                                                     \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : [r] "=&r"(r)                                         \
            : [n] "r"(n), [b] "r"(line)                            \
            : "r8", "r9", "r17", "r18", "r19", "r20", "r21",       \
              "r22", "r23", "r24", "ctr", "v0", "v1", "v2", "v3",  \
              "memory");                                           \
        t0 = timebase() - t0;                                      \
        sink += r;                                                 \
        return t0;                                                 \
    }

/* One family: its body at K = 0, 2, 4 and 8 addi. */
#define FAMILY(name, before, after)                                \
    CHAIN(name##_0, before ADDI0 after)                            \
    CHAIN(name##_2, before ADDI2 after)                            \
    CHAIN(name##_4, before ADDI4 after)                            \
    CHAIN(name##_8, before ADDI8 after)

FAMILY(chain_addi, "", "")
FAMILY(chain_hit, "ld 9,0(8)\n", "")
FAMILY(chain_store, "std 9,0(8)\n", "")
FAMILY(chain_reload, "std 9,0(8)\n", "ld 9,0(8)\n")
FAMILY(chain_mtvscr, "mtvscr 0\n", "vaddubm 1,2,3\n")

typedef u64 (*chain_fn)(u64);

static const chain_fn chains[CHAINS] = {
    chain_addi_0, chain_addi_2, chain_addi_4, chain_addi_8,
    chain_hit_0, chain_hit_2, chain_hit_4, chain_hit_8,
    chain_store_0, chain_store_2, chain_store_4, chain_store_8,
    chain_reload_0, chain_reload_2, chain_reload_4, chain_reload_8,
    chain_mtvscr_0, chain_mtvscr_2, chain_mtvscr_4, chain_mtvscr_8,
};

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
        chains[c](ITER);
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
    put(120 + loop, ticks);
    put(80 + loop, sib);
    put(160 + loop, counted);
    put(200 + loop, runs);
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
    for (i = 0; i < 16; i++)
        line[i] = i;

    for (c = 0; c < CHAINS; c++)
        timed(c, c);

    put(242, ~0ull);
    put(243, ~0ull);
    put(246, ~0ull);
    started = 0;
    stop = 0;
    rounds = 0;
    rc = cgov_ppu_thread_create(
        &tid, make_thread_param(&sibling_param, (const void *)&sibling_entry),
        0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
    put(242, (u64)(s64)rc);
    if (rc == 0) {
        u64 t0 = timebase();
        u64 before;
        while (!started && timebase() - t0 < START_WAIT_TB) {
        }
        put(245, started);
        before = rounds;
        t0 = timebase();
        put(246, (u64)(s64)syscall1_s32(SYS_TIMER_USLEEP, ALONE_US));
        alone_ticks = timebase() - t0;
        alone_rounds = rounds - before;
        put(241, alone_ticks);
        put(240, alone_rounds);
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS + c, c);
        stop = 1;
        put(243, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(244, retval);
    }
    put(247, sink);
    put(248, REPS);
    put(249, ITER);
    put(250, 0x5050534bull);

    write_frame();
    return 0;
}
