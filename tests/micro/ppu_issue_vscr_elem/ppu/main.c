/* PPU probe: which VMX element stores and single-precision FP loads an
 * mtvscr holds past its fence, and for how long. Each loop runs a fixed
 * body ITER times between two time-base reads. The probe runs each loop
 * REPS times and records the smallest and the largest delta. The loop
 * branch is bdnz. Every vector register starts zero, so mtvscr v0 writes
 * a zero VSCR. Each load and store addresses one 128-byte-aligned buffer
 * the probe zeroes first; lfsu with a zero displacement leaves its base
 * unchanged. No loop writes v20, v21 or f2, and nothing reads a loaded
 * value.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mtvscr v0; stvebx v20,0,p
 *   1  mtvscr v0; stvehx v20,0,p
 *   2  mtvscr v0; stvewx v20,0,p
 *   3  mtvscr v0; stvx v21,0,p
 *   4  mtvscr v0; lfs f1,0(p)
 *   5  mtvscr v0; lfsx f1,0,p
 *   6  mtvscr v0; lfsu f1,0(p)
 *   7  mtvscr v0; lfd f1,0(p)
 *   8  mtvscr v0; stvebx v20,0,p; stvebx v21,0,p
 *   9  mtvscr v0; lfs f1,0(p); lfs f3,0(p)
 *  10  mtvscr v0; lvebx v1,0,p
 *  11  mtvscr v0
 *   word
 *  24  REPS
 *  25  log2 of ITER
 *  26  0x50505645 ("PPVE"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     27
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 18

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

#define MTVSCR  "mtvscr 0\n"
#define STVEBX  "stvebx 20,0,%[p]\n"
#define STVEBX2 "stvebx 21,0,%[p]\n"
#define STVEHX  "stvehx 20,0,%[p]\n"
#define STVEWX  "stvewx 20,0,%[p]\n"
#define STVX    "stvx 21,0,%[p]\n"
#define LFS     "lfs 1,0(%[p])\n"
#define LFS3    "lfs 3,0(%[p])\n"
#define LFSX    "lfsx 1,0,%[p]\n"
#define LFSU    "lfsu 1,0(%[p])\n"
#define LFD     "lfd 1,0(%[p])\n"
#define LVEBX   "lvebx 1,0,%[p]\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        void *p_ = buf;                                            \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : [p] "+b"(p_) : [n] "r"(n)                            \
            : "ctr", "memory", "v0", "v1", "fr1", "fr3");          \
        return timebase() - t0;                                    \
    }

MLOOP(loop_stvebx, MTVSCR STVEBX)
MLOOP(loop_stvehx, MTVSCR STVEHX)
MLOOP(loop_stvewx, MTVSCR STVEWX)
MLOOP(loop_stvx, MTVSCR STVX)
MLOOP(loop_lfs, MTVSCR LFS)
MLOOP(loop_lfsx, MTVSCR LFSX)
MLOOP(loop_lfsu, MTVSCR LFSU)
MLOOP(loop_lfd, MTVSCR LFD)
MLOOP(loop_stvebx2, MTVSCR STVEBX STVEBX2)
MLOOP(loop_lfs2, MTVSCR LFS LFS3)
MLOOP(loop_lvebx, MTVSCR LVEBX)
MLOOP(loop_mtvscr, MTVSCR)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* Every vector register the loops read starts zero. */
    __asm__ volatile (
        "vxor 0,0,0\n" "vxor 20,20,20\n" "vxor 21,21,21\n"
        : : : "v0", "v20", "v21");

    TIMED(0, loop_stvebx(n));
    TIMED(1, loop_stvehx(n));
    TIMED(2, loop_stvewx(n));
    TIMED(3, loop_stvx(n));
    TIMED(4, loop_lfs(n));
    TIMED(5, loop_lfsx(n));
    TIMED(6, loop_lfsu(n));
    TIMED(7, loop_lfd(n));
    TIMED(8, loop_stvebx2(n));
    TIMED(9, loop_lfs2(n));
    TIMED(10, loop_lvebx(n));
    TIMED(11, loop_mtvscr(n));

    put(24, REPS);
    put(25, ITER_LOG2);
    put(26, 0x50505645ull);

    write_frame();
    return 0;
}
