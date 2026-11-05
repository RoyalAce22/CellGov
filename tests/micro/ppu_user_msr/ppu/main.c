/* PPU probe: what the MSR a PS3 user thread runs under shows to the
 * thread, on the main thread and on one created thread.
 *
 * Problem-state code cannot read the MSR, so each thread records what
 * its bits govern:
 *   - its first floating-point and its first vector instruction, at the
 *     thread's entry (MSR[FP], MSR[VEC]),
 *   - addic of 0xFFFF_FFFF + 1 and add. giving 0x8000_0000 (MSR[SF]: CA
 *     out of bit 0 and CR0 from all 64 bits in 64-bit mode),
 *   - last, fdiv of 0 / 0 with FPSCR[VE] = 1 (MSR[FE0:FE1]: whether the
 *     thread survives, the FPSCR, and whether the target changed).
 * The created thread runs its whole block before main runs its fdiv.
 *
 * Payload: big-endian u64 words. Thread t's block starts at word 10t
 * (t = 0 main, t = 1 the created thread).
 *   +0  the first FP result: fadd 1.0 + 2.0 (bits)
 *   +1  the first vector result: word 0 of vaddubm 0x01.. + 0x02..
 *   +2  addic r, 0xFFFF_FFFF, 1: the result
 *   +3  the XER after it (mfxer)
 *   +4  add. of 0x7FFF_FFFF + 1: the result
 *   +5  the CR after it (mfcr)
 *   +6  after fdiv 0 / 0 with VE = 1: the FPSCR (mffs, low word)
 *   +7  the fdiv target, preset to 0x123456789ABCDEF0
 *   +8  1 when the thread ran past the fdiv
 *   +9  0x5555 + t: the thread wrote its block
 *  20  sc 52 thread create rc
 *  21  sc 44 join rc
 *  22  the created thread's exit value (expected 0x4D53)
 *  23  0x50504D53 ("PPMS"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT 41
#define SYS_PPU_THREAD_JOIN 44

#define WORDS   24
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static const u64 ONE = 0x3FF0000000000000ull;
static const u64 TWO = 0x4000000000000000ull;
static const u64 ZERO = 0;
static const u64 SENTINEL = 0x123456789ABCDEF0ull;
static const unsigned int BYTES_1[4] __attribute__((aligned(16))) =
    { 0x01010101, 0x01010101, 0x01010101, 0x01010101 };
static const unsigned int BYTES_2[4] __attribute__((aligned(16))) =
    { 0x02020202, 0x02020202, 0x02020202, 0x02020202 };

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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

static struct cg_thread_param child_param __attribute__((aligned(8)));

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

/* The thread's first FP and first vector instruction. Each thread calls
 * this before any other FP or vector use. */
static void __attribute__((noinline)) first_fp_and_vector(unsigned base)
{
    u64 sum = 0;
    unsigned int vec[4] __attribute__((aligned(16)));
    __asm__ volatile (
        "lfd 1, 0(%1)\n"
        "lfd 2, 0(%2)\n"
        "fadd 1, 1, 2\n"
        "stfd 1, 0(%0)\n"
        : : "b"(&sum), "b"(&ONE), "b"(&TWO)
        : "fr1", "fr2", "memory");
    __asm__ volatile (
        "lvx 1, 0, %1\n"
        "lvx 2, 0, %2\n"
        "vaddubm 3, 1, 2\n"
        "stvx 3, 0, %0\n"
        : : "b"(vec), "b"(BYTES_1), "b"(BYTES_2)
        : "v1", "v2", "v3", "memory");
    put(base + 0, sum);
    put(base + 1, vec[0]);
}

/* addic and add. across the 32-bit boundary. */
static void __attribute__((noinline)) mode_cases(unsigned base)
{
    u64 r, xer, cr;
    __asm__ volatile (
        "li 0, 0\n"
        "mtxer 0\n"
        "lis %0, 0\n"
        "ori %0, %0, 0xFFFF\n"
        "oris %0, %0, 0xFFFF\n"
        "addic %0, %0, 1\n"
        "mfxer %1\n"
        : "=&r"(r), "=&r"(xer) : : "r0", "xer");
    put(base + 2, r);
    put(base + 3, xer);
    __asm__ volatile (
        "lis %0, 0x7FFF\n"
        "ori %0, %0, 0xFFFF\n"
        "li 0, 1\n"
        "add. %0, %0, 0\n"
        "mfcr %1\n"
        : "=&r"(r), "=&r"(cr) : : "r0", "cr0");
    put(base + 4, r);
    put(base + 5, cr);
}

/* fdiv 0 / 0 with FPSCR[VE] = 1. On the console an enabled invalid
 * operation in Ignore Exceptions mode runs on; under any other mode the
 * thread may end here, so it runs last. */
static void __attribute__((noinline)) enabled_invalid_case(unsigned base)
{
    u64 target = 0, fpscr = 0;
    __asm__ volatile (
        "lfd 1, 0(%2)\n"
        "lfd 3, 0(%3)\n"
        "mtfsb1 24\n"
        "fdiv 3, 1, 1\n"
        "mffs 4\n"
        "mtfsb0 24\n"
        "stfd 3, 0(%0)\n"
        "stfd 4, 0(%1)\n"
        : : "b"(&target), "b"(&fpscr), "b"(&ZERO), "b"(&SENTINEL)
        : "fr1", "fr3", "fr4", "memory");
    put(base + 6, fpscr & 0xFFFFFFFFull);
    put(base + 7, target);
    put(base + 8, 1);
}

static void child_entry(void *arg)
{
    (void)arg;
    first_fp_and_vector(10);
    mode_cases(10);
    enabled_invalid_case(10);
    put(19, 0x5556);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x4D53);
    for (;;) { }
}

int main(void)
{
    unsigned long long tid = 0, retval = 0;
    s32 rc;

    /* `out` is zero-filled static storage, so nothing runs before the
     * thread's first FP and vector instructions. */
    first_fp_and_vector(0);
    mode_cases(0);

    rc = cgov_ppu_thread_create(
        &tid,
        make_thread_param(&child_param, (const void *)&child_entry),
        0, 0, 2000, 0x4000, THREAD_JOINABLE, 0);
    put(20, (u64)(s64)rc);
    if (rc == 0) {
        put(21, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(22, retval);
    }

    enabled_invalid_case(0);
    put(9, 0x5555);
    put(23, 0x50504D53ull);

    write_frame();
    return 0;
}
