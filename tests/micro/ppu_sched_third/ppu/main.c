/* PPU probe: how LV2 shares the PPE's two hardware threads among three
 * runnable PPU threads. Each phase starts up to three spinner threads,
 * runs them for a window, then stops and joins them. A spinner runs
 * rounds of 512 independent addi and records, from the time base:
 * its rounds, when its first and last rounds began, and the gaps
 * between rounds longer than GAP_TB (time it did not run).
 *
 *   phase  spinners (priority)   main thread       spinner 0
 *   0      1500 1500 1500        sleeps 2 s        spins
 *   1      1400 1500 1600        sleeps 2 s        spins
 *   2      1500 1500 1500        sleeps 2 s        yields after each round
 *   3      1500 1500 1500        sleeps 2 s        sleeps 1 ms each 1024 rounds
 *   4      2000 2000             spins 5 s         spins
 *   5      1001 1001             spins 5 s         spins
 *
 * The main thread runs at priority 1001, at or above every spinner, so
 * it can always stop them.
 *
 * Payload: big-endian u64 words. Spinner k of phase p has eight words
 * from 8 * (3p + k):
 *   +0 create rc (all ones: not created)   +1 join rc (all ones: not joined)
 *   +2 exit value (expected 0x5157)        +3 rounds
 *   +4 its first round, in time-base ticks after the phase began
 *   +5 its last round, likewise            +6 gaps longer than GAP_TB
 *   +7 its longest gap
 * Word 144 + p: the phase's window in time-base ticks. Word 150: GAP_TB.
 * Word 151: 0x50505354 ("PPST"): the probe wrote every word.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define MAIN_PRIORITY 1001

SYS_PROCESS_PARAM(MAIN_PRIORITY, 0x10000)

#define SYS_PPU_THREAD_EXIT  41
#define SYS_PPU_THREAD_YIELD 43
#define SYS_PPU_THREAD_JOIN  44
#define SYS_TIMER_USLEEP     141

#define PHASES   6
#define PER      3
#define SLOTS    (PHASES * PER)
#define WORDS    152
#define PAYLOAD  (WORDS * 8)

#define TB_HZ        79800000ull
/* A gap is a round that began more than 100 us after the one before. */
#define GAP_TB       (TB_HZ / 10000ull)
/* A spinner stops at its phase's stop flag or 10 s after it starts. */
#define LIMIT_TB     (TB_HZ * 10ull)

enum { SPIN, YIELD, NAP };

struct spinner {
    u64 rounds;
    u64 first;
    u64 last;
    u64 gaps;
    u64 max_gap;
};

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile struct spinner spin[SLOTS];
static volatile unsigned int stop;
static volatile u64 phase_start;
static int mode[SLOTS];
static s32 create_rc[SLOTS];

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R4(x) x x x x
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

static struct cg_thread_param params[SLOTS] __attribute__((aligned(8)));
static unsigned long long tids[SLOTS];

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

/* A spinner: rounds of independent addi until the stop flag or its time
 * limit. `arg` is its slot. */
static void spinner_entry(void *arg)
{
    unsigned k = (unsigned)(unsigned long)arg;
    u64 start = timebase();
    u64 prev = 0;
    spin[k].first = start - phase_start;
    while (!stop && timebase() - start < LIMIT_TB) {
        u64 now = timebase();
        if (prev != 0 && now - prev > GAP_TB) {
            spin[k].gaps = spin[k].gaps + 1;
            if (now - prev > spin[k].max_gap)
                spin[k].max_gap = now - prev;
        }
        prev = now;
        spin[k].last = now - phase_start;
        __asm__ volatile (
            "li 14,32\n"
            "mtctr 14\n"
            "1:\n"
            R8("addi 15,15,1\n" "addi 16,16,1\n")
            "bdnz 1b\n"
            : : : "r14", "r15", "r16", "ctr");
        spin[k].rounds = spin[k].rounds + 1;
        if (mode[k] == YIELD)
            (void)syscall0_s32(SYS_PPU_THREAD_YIELD);
        else if (mode[k] == NAP && (spin[k].rounds & 1023) == 0)
            (void)syscall1_s32(SYS_TIMER_USLEEP, 1000);
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

/* Run phase `p`: `n` spinners at `prio[k]`, spinner 0 in `first_mode`;
 * the main thread sleeps or spins for `window_us`. */
static void phase(unsigned p, unsigned n, const u64 *prio, int first_mode,
                  int main_spins, u64 window_us)
{
    unsigned k;
    u64 t0;
    stop = 0;
    phase_start = timebase();
    for (k = 0; k < n; k++) {
        unsigned s = PER * p + k;
        mode[s] = k == 0 ? first_mode : SPIN;
        create_rc[s] = cgov_ppu_thread_create(
            &tids[s], make_thread_param(&params[s], (const void *)&spinner_entry),
            s, 0, prio[k], 0x4000, THREAD_JOINABLE, 0);
        put(8 * s, (u64)(s64)create_rc[s]);
    }
    if (main_spins) {
        t0 = timebase();
        while (timebase() - t0 < window_us * (TB_HZ / 1000000ull)) {
        }
    } else {
        (void)syscall1_s32(SYS_TIMER_USLEEP, window_us);
    }
    stop = 1;
    put(144 + p, timebase() - phase_start);
    for (k = 0; k < n; k++) {
        unsigned s = PER * p + k;
        unsigned long long retval = 0;
        if (create_rc[s] != 0)
            continue;
        put(8 * s + 1, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tids[s],
                                               (unsigned long)&retval));
        put(8 * s + 2, retval);
        put(8 * s + 3, spin[s].rounds);
        put(8 * s + 4, spin[s].first);
        put(8 * s + 5, spin[s].last);
        put(8 * s + 6, spin[s].gaps);
        put(8 * s + 7, spin[s].max_gap);
    }
}

int main(void)
{
    unsigned i;
    static const u64 equal[PER] = { 1500, 1500, 1500 };
    static const u64 graded[PER] = { 1400, 1500, 1600 };
    static const u64 lower[PER] = { 2000, 2000, 2000 };
    static const u64 same[PER] = { MAIN_PRIORITY, MAIN_PRIORITY, MAIN_PRIORITY };

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < SLOTS; i++) {
        create_rc[i] = -1;
        put(8 * i, ~0ull);
        put(8 * i + 1, ~0ull);
    }

    phase(0, 3, equal, SPIN, 0, 2000000);
    phase(1, 3, graded, SPIN, 0, 2000000);
    phase(2, 3, equal, YIELD, 0, 2000000);
    phase(3, 3, equal, NAP, 0, 2000000);
    phase(4, 2, lower, SPIN, 1, 5000000);
    phase(5, 2, same, SPIN, 1, 5000000);

    put(150, GAP_TB);
    put(151, 0x50505354ull);

    write_frame();
    return 0;
}
