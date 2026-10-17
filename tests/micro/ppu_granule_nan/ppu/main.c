/* PPU probe: three PPU facts a user program can see -- the NaNs the FPU
 * generates and propagates, how many bytes one dcbz zeroes, and how far
 * from a reserved address another thread's store must land to leave the
 * reservation standing.
 *
 * Floating point: each case loads its operands from memory into FPRs and
 * runs one instruction under the process's default FPSCR. Operands:
 * INF = 0x7FF0000000000000, QNAN_A = 0x7FF8000000000AAA,
 * QNAN_C = 0xFFF8000000000BBB, SNAN_B = 0x7FF0000000000123.
 *
 * dcbz: a 512-byte, 128-byte-aligned buffer filled with 0xEE; one dcbz
 * on buffer + 128 + 40.
 *
 * Reservation: main does lwarx on a 128-byte-aligned line, raises a flag,
 * and spins until a second thread has stored one word at line + OFFSET,
 * then does stwcx. on the line. OFFSET is 64 (the same 128-byte line),
 * 128 (the next line) or 0x1000 (a control far away). TRIALS trials per
 * offset; a trial whose store did not arrive within the spin bound is a
 * timeout and its stwcx. result is not counted.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2 + 2c, case c (high word, low word of the result FPR):
 *     0 fdiv 0/0          1 fdiv INF/INF        2 fsub INF-INF
 *     3 fadd INF+-INF     4 fmul 0*INF          5 fsqrt -1.0
 *     6 fmadd INF*0+1.0   7 fdivs 0/0           8 fadds INF+-INF
 *     9 fsqrts -1.0      10 fmadd 1.0*1.0+SNAN_B
 *    11 fadd SNAN_B+1.0  12 fadd QNAN_C+QNAN_A  13 fadd QNAN_A+QNAN_C
 *  30  FPSCR low word before the cases   31  after them
 *  32  offset of the first zeroed byte (0xFFFFFFFF = none)
 *  33  bytes zeroed   34  of them outside [128, 256)
 *  35 + 3k, offset k (0: 64, 1: 128, 2: 0x1000): stwcx. successes,
 *       failures, timeouts
 *  44  TRIALS
 */

#include <string.h>

#include <sys/process.h>
#include <lv2/thread.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 45
#define TRIALS 32
#define SPIN_TICKS 800000ull

#define SYS_PPU_THREAD_EXIT 41
#define SYS_PPU_THREAD_JOIN 44
#define THREAD_JOINABLE 1
#define CHILD_PRIO 1000
#define STACK_SIZE 0x4000

#define INF    0x7FF0000000000000ull
#define NEGINF 0xFFF0000000000000ull
#define QNAN_A 0x7FF8000000000AAAull
#define QNAN_C 0xFFF8000000000BBBull
#define SNAN_B 0x7FF0000000000123ull
#define ONE    0x3FF0000000000000ull
#define NEGONE 0xBFF0000000000000ull

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned char block[512] __attribute__((aligned(128)));
static volatile unsigned int lines[0x1100 / 4] __attribute__((aligned(128)));
static volatile unsigned int flag_a __attribute__((aligned(128)));
static volatile unsigned int flag_b __attribute__((aligned(128)));
static volatile unsigned int b_offset __attribute__((aligned(128)));
static volatile unsigned int b_stop;
static volatile unsigned long long opnd[3] __attribute__((aligned(8)));
static volatile unsigned long long fout __attribute__((aligned(8)));

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result[0] = step;
    result[1] = (unsigned int)rc;
    write_frame();
    return (int)step;
}

static inline unsigned long long timebase(void)
{
    unsigned long long tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static unsigned int fpscr_low(void)
{
    double f;
    unsigned long long bits;
    __asm__ volatile ("mffs %0\n\tstfd %0,0(%1)" : "=&f"(f) : "b"(&fout) : "memory");
    bits = fout;
    return (unsigned int)bits;
}

static void put_case(unsigned int c)
{
    result[2 + 2 * c] = (unsigned int)(fout >> 32);
    result[3 + 2 * c] = (unsigned int)fout;
}

/* One FP instruction: operands a, b (and c) from memory, result stored
 * to fout. Each macro names the instruction and its operand order. */
#define FP2(c, insn, x, y)                                                   \
    do {                                                                     \
        double fa, fb, ft;                                                   \
        opnd[0] = (x); opnd[1] = (y);                                        \
        __asm__ volatile ("lfd %1,0(%3)\n\tlfd %2,8(%3)\n\t"                 \
                          insn " %0,%1,%2\n\tstfd %0,0(%4)"                  \
                          : "=&f"(ft), "=&f"(fa), "=&f"(fb)                  \
                          : "b"(opnd), "b"(&fout) : "memory");               \
        put_case(c);                                                         \
    } while (0)

#define FP1(c, insn, y)                                                      \
    do {                                                                     \
        double fb, ft;                                                       \
        opnd[0] = (y);                                                       \
        __asm__ volatile ("lfd %1,0(%2)\n\t" insn " %0,%1\n\tstfd %0,0(%3)"  \
                          : "=&f"(ft), "=&f"(fb)                             \
                          : "b"(opnd), "b"(&fout) : "memory");               \
        put_case(c);                                                         \
    } while (0)

/* fmadd frt = fra * frc + frb. */
#define FMADD(c, fra_v, frc_v, frb_v)                                        \
    do {                                                                     \
        double fa, fb, fc, ft;                                               \
        opnd[0] = (fra_v); opnd[1] = (frc_v); opnd[2] = (frb_v);             \
        __asm__ volatile ("lfd %1,0(%4)\n\tlfd %2,8(%4)\n\tlfd %3,16(%4)\n\t"\
                          "fmadd %0,%1,%2,%3\n\tstfd %0,0(%5)"               \
                          : "=&f"(ft), "=&f"(fa), "=&f"(fc), "=&f"(fb)       \
                          : "b"(opnd), "b"(&fout) : "memory");               \
        put_case(c);                                                         \
    } while (0)

static void fp_cases(void)
{
    result[30] = fpscr_low();
    FP2(0, "fdiv", 0ull, 0ull);
    FP2(1, "fdiv", INF, INF);
    FP2(2, "fsub", INF, INF);
    FP2(3, "fadd", INF, NEGINF);
    FP2(4, "fmul", 0ull, INF);
    FP1(5, "fsqrt", NEGONE);
    FMADD(6, INF, 0ull, ONE);
    FP2(7, "fdivs", 0ull, 0ull);
    FP2(8, "fadds", INF, NEGINF);
    FP1(9, "fsqrts", NEGONE);
    FMADD(10, ONE, ONE, SNAN_B);
    FP2(11, "fadd", SNAN_B, ONE);
    FP2(12, "fadd", QNAN_C, QNAN_A);
    FP2(13, "fadd", QNAN_A, QNAN_C);
    result[31] = fpscr_low();
}

static void dcbz_case(void)
{
    unsigned int i, first = 0xFFFFFFFFu, zeroed = 0, outside = 0;
    for (i = 0; i < sizeof(block); i++)
        block[i] = 0xEE;
    __sync_synchronize();
    __asm__ volatile ("dcbz 0,%0" : : "r"(&block[128 + 40]) : "memory");
    __sync_synchronize();
    for (i = 0; i < sizeof(block); i++) {
        if (block[i] == 0) {
            if (first == 0xFFFFFFFFu)
                first = i;
            zeroed++;
            if (i < 128 || i >= 256)
                outside++;
        }
    }
    result[32] = first;
    result[33] = zeroed;
    result[34] = outside;
}

/* The toolchain's &fn is a 24-byte ELFv1 descriptor; syscall 52's
 * thread-init block names an 8-byte { u32 code; u32 toc } one. */
struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static unsigned int self_tls(void)
{
    unsigned long r13;
    __asm__ volatile ("mr %0, 13" : "=r"(r13));
    return (unsigned int)r13;
}

static u64 make_param(struct thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = self_tls();
    return (u64)(unsigned long)p;
}

/* The storing thread: for each trial main raises flag_a to, store one
 * word at line + b_offset, then answer on flag_b. */
static void storer(u64 arg)
{
    unsigned int seen = 0;
    (void)arg;
    while (!b_stop) {
        unsigned int t = flag_a;
        if (t != seen) {
            seen = t;
            lines[b_offset / 4] = t;
            __sync_synchronize();
            flag_b = t;
            __sync_synchronize();
        }
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* One trial: 1 when stwcx. succeeded, 0 when it failed, 2 on timeout. */
static unsigned int trial(unsigned int t)
{
    unsigned int v, cr, arrived = 0;
    unsigned long long t0;
    __asm__ volatile ("lwarx %0,0,%1" : "=r"(v) : "r"(&lines[0]) : "memory");
    flag_a = t;
    t0 = timebase();
    while (timebase() - t0 < SPIN_TICKS) {
        if (flag_b == t) {
            arrived = 1;
            break;
        }
    }
    __asm__ volatile ("stwcx. %2,0,%1\n\tmfcr %0"
                      : "=r"(cr) : "r"(&lines[0]), "r"(v + 1) : "cr0", "memory");
    if (!arrived)
        return 2;
    return (cr >> 29) & 1;
}

static struct thread_param param_b __attribute__((aligned(8)));

int main(void)
{
    static const unsigned int offsets[3] = { 64, 128, 0x1000 };
    unsigned long long tid, value;
    unsigned int rc, k, i, t = 0;

    memset((void *)result, 0, sizeof(result));
    result[44] = TRIALS;

    fp_cases();
    dcbz_case();

    flag_a = 0;
    flag_b = 0;
    b_stop = 0;
    b_offset = offsets[0];
    __sync_synchronize();
    rc = (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, (u64)(unsigned long)&tid,
                                    make_param(&param_b, (const void *)&storer), 0, 0,
                                    CHILD_PRIO, STACK_SIZE, THREAD_JOINABLE, 0);
    if (rc != 0)
        return fail(1, (int)rc);
    rc = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0);
    if (rc != 0)
        return fail(2, (int)rc);

    for (k = 0; k < 3; k++) {
        b_offset = offsets[k];
        __sync_synchronize();
        for (i = 0; i < TRIALS; i++) {
            unsigned int r = trial(++t);
            result[35 + 3 * k + (r == 1 ? 0 : r == 0 ? 1 : 2)]++;
        }
    }

    b_stop = 1;
    __sync_synchronize();
    rc = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (u64)(unsigned long)&value);
    if (rc != 0)
        return fail(3, (int)rc);

    write_frame();
    return 0;
}
