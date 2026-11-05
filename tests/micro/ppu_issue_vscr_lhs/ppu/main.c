/* PPU probe: what a load that hits a store in the store queue costs by
 * load and store form, and how an mtvscr before the store changes it.
 * Each loop runs a fixed body ITER times between two time-base reads.
 * The probe runs each loop REPS times and records the smallest and the
 * largest delta. The loop branch is bdnz. Every vector register starts
 * zero, so mtvscr v0 writes a zero VSCR. Each load and store addresses
 * the first bytes of one 128-byte-aligned buffer the probe zeroes first.
 * Nothing in a loop reads a loaded value.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  stw r14,0(p); lwz r19,0(p)
 *   1  std r14,0(p); lfd f1,0(p)
 *   2  stfd f2,0(p); lfd f1,0(p)
 *   3  stw r14,0(p); lfs f1,0(p)
 *   4  stw r14,0(p); lfd f1,0(p)
 *   5  mtvscr v0; stw r14,0(p); lwz r19,0(p)
 *   6  mtvscr v0; std r14,0(p); lfd f1,0(p)
 *   7  mtvscr v0; stfd f2,0(p); lfd f1,0(p)
 *   8  mtvscr v0; lwz r19,0(p)
 *   9  mtvscr v0; stw r14,0(p)
 *   word
 *  20  REPS
 *  21  log2 of ITER
 *  22  0x5050564c ("PPVL"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     23
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 16

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static unsigned char buf[128] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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

/* Run `expr`, one timed loop, REPS times; keep the smallest delta in
 * word 2 * loop and the largest in the word after it. */
#define TIMED(loop, expr)                              \
    do {                                               \
        u64 lo_ = ~0ull, hi_ = 0;                      \
        unsigned r_;                                   \
        for (r_ = 0; r_ < REPS; r_++) {                \
            u64 t_ = (expr);                           \
            if (t_ < lo_)                              \
                lo_ = t_;                              \
            if (t_ > hi_)                              \
                hi_ = t_;                              \
        }                                              \
        put(2 * (loop), lo_);                          \
        put(2 * (loop) + 1, hi_);                      \
    } while (0)

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

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define MTVSCR "mtvscr 0\n"
#define STW  "stw 14,0(%[p])\n"
#define STD  "std 14,0(%[p])\n"
#define STFD "stfd 2,0(%[p])\n"
#define LWZ  "lwz 19,0(%[p])\n"
#define LFD  "lfd 1,0(%[p])\n"
#define LFS  "lfs 1,0(%[p])\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n), [p] "b"(buf)                           \
            : "r14", "r19", "ctr", "memory", "v0", "fr1", "fr2");  \
        return timebase() - t0;                                    \
    }

MLOOP(loop_stw_lwz, STW LWZ)
MLOOP(loop_std_lfd, STD LFD)
MLOOP(loop_stfd_lfd, STFD LFD)
MLOOP(loop_stw_lfs, STW LFS)
MLOOP(loop_stw_lfd, STW LFD)
MLOOP(loop_fenced_stw_lwz, MTVSCR STW LWZ)
MLOOP(loop_fenced_std_lfd, MTVSCR STD LFD)
MLOOP(loop_fenced_stfd_lfd, MTVSCR STFD LFD)
MLOOP(loop_fenced_lwz, MTVSCR LWZ)
MLOOP(loop_fenced_stw, MTVSCR STW)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* v0, r14 and f2 start zero. f2 is loaded from the zeroed buffer. */
    __asm__ volatile (
        "vxor 0,0,0\n" "li 14,0\n" "lfd 2,0(%[p])\n"
        : : [p] "b"(buf) : "v0", "r14", "fr2", "memory");

    TIMED(0, loop_stw_lwz(n));
    TIMED(1, loop_std_lfd(n));
    TIMED(2, loop_stfd_lfd(n));
    TIMED(3, loop_stw_lfs(n));
    TIMED(4, loop_stw_lfd(n));
    TIMED(5, loop_fenced_stw_lwz(n));
    TIMED(6, loop_fenced_std_lfd(n));
    TIMED(7, loop_fenced_stfd_lfd(n));
    TIMED(8, loop_fenced_lwz(n));
    TIMED(9, loop_fenced_stw(n));

    put(20, REPS);
    put(21, ITER_LOG2);
    put(22, 0x5050564cull);

    write_frame();
    return 0;
}
