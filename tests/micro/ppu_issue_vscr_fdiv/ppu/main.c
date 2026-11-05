/* PPU probe: what an FP divide or square root costs when an mtvscr fence
 * holds it, and whether fixed-point instructions after it issue past it.
 * Each loop runs a fixed body ITER times between two time-base reads. The
 * probe runs each loop REPS times and records the smallest and the
 * largest delta. The loop branch is bdnz. Before its loop each function
 * loads f20 = 1.0, f21 = 3.0 and f22 = 2.0 and zeroes v0, v20 and v21, so
 * mtvscr v0 writes a zero VSCR.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  fdiv f1,f20,f21
 *   1  mtvscr v0; fdiv f1,f20,f21
 *   2  mtvscr v0; fdiv f1,f20,f21; 16 independent addi
 *   3  fdiv f1,f20,f21; fdiv f2,f20,f21
 *   4  mtvscr v0; fdiv f1,f20,f21; fdiv f2,f20,f21
 *   5  mtvscr v0; fdiv f1,f20,f21; fdiv f2,f20,f21; 16 independent addi
 *   6  mtvscr v0; fsqrt f1,f22
 *   7  mtvscr v0; fsqrt f1,f22; 16 independent addi
 *   8  mtvscr v0; fdiv f1,f20,f21; fadd f3,f1,f1
 *   9  mtvscr v0; vaddubm v1,v20,v21; fdiv f1,f20,f21; 16 independent addi
 *   word
 *  20  REPS
 *  21  log2 of ITER
 *  22  0x50505646 ("PPVF"): the probe wrote every word
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

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

/* The FP inputs: f20, f21 and f22. */
static const double fin[3] __attribute__((aligned(8))) = { 1.0, 3.0, 2.0 };

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

/* The inputs load before the loop, outside its body. */
#define SETUP                                                   \
    "lfd 20,0(%[f])\n" "lfd 21,8(%[f])\n" "lfd 22,16(%[f])\n"   \
    "vxor 0,0,0\n" "vxor 20,20,20\n" "vxor 21,21,21\n"
#define LOOP_BEGIN SETUP "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define R2(x) x x
#define R4(x) R2(x) R2(x)

#define MTVSCR "mtvscr 0\n"
#define FDIV1  "fdiv 1,20,21\n"
#define FDIV2  "fdiv 2,20,21\n"
#define FSQRT  "fsqrt 1,22\n"
#define FADD   "fadd 3,1,1\n"
#define VADD1  "vaddubm 1,20,21\n"
#define ADDI4                                                   \
    "addi 14,14,1\n" "addi 15,15,1\n"                           \
    "addi 16,16,1\n" "addi 17,17,1\n"

#define FLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n), [f] "b"(fin)                           \
            : "r14", "r15", "r16", "r17", "ctr", "fr1", "fr2",     \
              "fr3", "fr20", "fr21", "fr22", "v0", "v1", "v20",    \
              "v21", "memory");                                    \
        return timebase() - t0;                                    \
    }

FLOOP(loop_fdiv, FDIV1)
FLOOP(loop_fenced_fdiv, MTVSCR FDIV1)
FLOOP(loop_fenced_fdiv_addi16, MTVSCR FDIV1 R4(ADDI4))
FLOOP(loop_fdiv2, FDIV1 FDIV2)
FLOOP(loop_fenced_fdiv2, MTVSCR FDIV1 FDIV2)
FLOOP(loop_fenced_fdiv2_addi16, MTVSCR FDIV1 FDIV2 R4(ADDI4))
FLOOP(loop_fenced_fsqrt, MTVSCR FSQRT)
FLOOP(loop_fenced_fsqrt_addi16, MTVSCR FSQRT R4(ADDI4))
FLOOP(loop_fenced_fdiv_use, MTVSCR FDIV1 FADD)
FLOOP(loop_fenced_vadd_fdiv_addi16, MTVSCR VADD1 FDIV1 R4(ADDI4))

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    TIMED(0, loop_fdiv(n));
    TIMED(1, loop_fenced_fdiv(n));
    TIMED(2, loop_fenced_fdiv_addi16(n));
    TIMED(3, loop_fdiv2(n));
    TIMED(4, loop_fenced_fdiv2(n));
    TIMED(5, loop_fenced_fdiv2_addi16(n));
    TIMED(6, loop_fenced_fsqrt(n));
    TIMED(7, loop_fenced_fsqrt_addi16(n));
    TIMED(8, loop_fenced_fdiv_use(n));
    TIMED(9, loop_fenced_vadd_fdiv_addi16(n));

    put(20, REPS);
    put(21, ITER_LOG2);
    put(22, 0x50505646ull);

    write_frame();
    return 0;
}
