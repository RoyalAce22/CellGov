/* PPU probe: whether one PPU thread's mtvscr stalls the vector-scalar
 * work of the sibling thread beside it. Each phase starts one sibling
 * running one loop that counts its rounds in memory and reads no time
 * base, measures the sibling's rate while the main thread sleeps, then
 * times five chains on the main thread beside it. Phase 0 runs no
 * sibling. Every vector register either thread reads starts zero, so
 * mtvscr v0 writes a zero VSCR.
 *
 *   phase  sibling round: four passes of 8 instructions and a bdnz, then
 *          a load of the stop flag and an increment of the round count,
 *          the loop's head at a 128-byte line
 *   0      none
 *   1      independent vaddubm (four targets)
 *   2      mtvscr v0
 *   3      mtvscr v0 and 7 independent vaddubm
 *   4      independent addi    (four targets)
 *
 *   chain  timed loop (1 << 16 iterations, bdnz)
 *   0      8 independent vaddubm
 *   1      mtvscr v0
 *   2      mtvscr v0; 4 independent vaddubm
 *   3      16 independent addi (four targets)
 *   4      8 dependent fadd
 *
 * The main thread runs at priority 1001 and the sibling at 2000, so the
 * two hold the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 5 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   50 + L         the sibling's rounds while the loop's REPS runs ran
 *   75 + L         the time-base ticks those REPS runs took
 * Phase p:
 *   100 + 2p       the sibling's rounds while the main thread slept
 *   101 + 2p       the time-base ticks of that sleep
 *   110 + p        the sibling's create rc (all ones: none created)
 *   115 + p        its join rc (all ones: not joined)
 *   120 + p        its exit value (expected 0x5157)
 *   125 + p        1 when it reported it runs
 * Word 130: the sum of every chain's result (keeps each loop live).
 * Word 131: REPS. Word 132: 0x50505753 ("PPWS"): the probe wrote every
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

#define PHASES  5
#define CHAINS  5
#define WORDS   136
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define ITER    (1u << 16)
#define DONE    0x50505753ull

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

#define VZERO "vxor 0,0,0\n" "vxor 20,20,20\n" "vxor 21,21,21\n"

#define VADD4                                                   \
    "vaddubm 1,20,21\n" "vaddubm 2,20,21\n"                     \
    "vaddubm 3,20,21\n" "vaddubm 4,20,21\n"
#define VADD7 VADD4 "vaddubm 5,20,21\n" "vaddubm 6,20,21\n" "vaddubm 7,20,21\n"
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
          "cr0", "memory", "v0", "v1", "v2", "v3", "v4", "v5",     \
          "v6", "v7", "v20", "v21")

static void sibling_entry(void *arg)
{
    unsigned k = kind;
    (void)arg;
    started = 1;
    switch (k) {
    case 1:
        SIB(R2(VADD4));
        break;
    case 2:
        SIB(R8("mtvscr 0\n"));
        break;
    case 3:
        SIB("mtvscr 0\n" VADD7);
        break;
    default:
        SIB(R2(ADDI4));
        break;
    }
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

VLOOP(chain_vadd8, VADD4 "vaddubm 5,20,21\n" "vaddubm 6,20,21\n"
                   "vaddubm 7,20,21\n" "vaddubm 8,20,21\n")
VLOOP(chain_mtvscr, "mtvscr 0\n")
VLOOP(chain_mtvscr_vadd, "mtvscr 0\n" VADD4)
VLOOP(chain_addi, R4(ADDI4))

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

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_vadd8(ITER);
    case 1: return chain_mtvscr(ITER);
    case 2: return chain_mtvscr_vadd(ITER);
    case 3: return chain_addi(ITER);
    default: return chain_fadd(ITER);
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
    put(75 + loop, timebase() - t0);
    put(50 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(110 + p, ~0ull);
        put(115 + p, ~0ull);
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
        put(110 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(125 + p, started);
            before = rounds;
            t0 = timebase();
            syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(101 + 2 * p, timebase() - t0);
            put(100 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(115 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(120 + p, retval);
    }
    put(130, sink);
    put(131, REPS);
    put(132, DONE);

    write_frame();
    return 0;
}
