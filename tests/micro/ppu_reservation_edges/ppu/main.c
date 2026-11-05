/* PPU probe: store-conditional results the architecture leaves undefined.
 *
 * The reserved line is the 128-byte-aligned start of a 256-byte buffer
 * nothing else touches. Every trial first writes SENT to every doubleword
 * of the buffer with ordinary stores, then runs its case and records
 * CR0[EQ] after the store-conditional and whether the store-conditional's
 * target now holds the value it stored. TRIALS trials per case, since an
 * undefined result may differ between executions.
 *
 * One thread, the load-reserve on line + 0 and the store-conditional in
 * one asm block:
 *   0  lwarx, stwcx. +0        1  lwarx, stwcx. +4
 *   2  lwarx, stwcx. +64       3  lwarx, stwcx. +124
 *   4  lwarx, stwcx. +128 (the next line)
 *   5  ldarx, stdcx. +0        6  ldarx, stdcx. +8
 *   7  ldarx, stdcx. +64       8  ldarx, stdcx. +120
 *   9  ldarx, stdcx. +128
 *  10  lwarx, stdcx. +0       11  ldarx, stwcx. +0
 *  12  lwarx, dcbf line, stwcx. +0
 *  13  lwarx, dcbst line, stwcx. +0
 * Two threads (the handshake of ppu_granule_nan): main does lwarx on line
 * + 0, raises flag_a, spins until the second thread has run its cache
 * operation on the line and answered on flag_b, then does stwcx. +0:
 *  14  no operation (control)  15  dcbst   16  dcbf   17  dcbtst
 * A two-thread trial whose answer did not arrive within the spin bound is
 * a timeout and its result is not counted.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  TRIALS
 *   3 + 3c, case c: CR0[EQ] set, target holds the stored value, timeouts
 */

#include <string.h>

#include <sys/process.h>
#include <lv2/thread.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 18
#define WORDS (3 + 3 * CASES)
#define TRIALS 32
#define SPIN_TICKS 800000ull
#define SENT 0xA5A5A5A5A5A5A5A5ull

#define SYS_PPU_THREAD_EXIT 41
#define SYS_PPU_THREAD_JOIN 44
#define THREAD_JOINABLE 1
#define CHILD_PRIO 1000
#define STACK_SIZE 0x4000

enum kind { WW, WD, DW, DD, W_DCBF_W, W_DCBST_W, TWO };

struct pair {
    enum kind kind;
    unsigned int off;
    unsigned int b_op;
};

static const struct pair CASE[CASES] = {
    { WW, 0, 0 },   { WW, 4, 0 },   { WW, 64, 0 },  { WW, 124, 0 }, { WW, 128, 0 },
    { DD, 0, 0 },   { DD, 8, 0 },   { DD, 64, 0 },  { DD, 120, 0 }, { DD, 128, 0 },
    { WD, 0, 0 },   { DW, 0, 0 },
    { W_DCBF_W, 0, 0 }, { W_DCBST_W, 0, 0 },
    { TWO, 0, 0 },  { TWO, 0, 1 },  { TWO, 0, 2 },  { TWO, 0, 3 },
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long buf[32] __attribute__((aligned(128)));
static volatile unsigned int flag_a __attribute__((aligned(128)));
static volatile unsigned int flag_b __attribute__((aligned(128)));
static volatile unsigned int b_op __attribute__((aligned(128)));
static volatile unsigned int b_stop;

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

static unsigned int run_pair(enum kind kind, unsigned long res, unsigned long st,
                             unsigned long long v)
{
    unsigned long long t;
    unsigned int cr = 0;
    switch (kind) {
    case WW:
        __asm__ volatile ("lwarx %1,0,%2\n\tstwcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t) : "r"(res), "r"(st), "r"(v) : "cr0", "memory");
        break;
    case WD:
        __asm__ volatile ("lwarx %1,0,%2\n\tstdcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t) : "r"(res), "r"(st), "r"(v) : "cr0", "memory");
        break;
    case DW:
        __asm__ volatile ("ldarx %1,0,%2\n\tstwcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t) : "r"(res), "r"(st), "r"(v) : "cr0", "memory");
        break;
    case DD:
        __asm__ volatile ("ldarx %1,0,%2\n\tstdcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t) : "r"(res), "r"(st), "r"(v) : "cr0", "memory");
        break;
    case W_DCBF_W:
        __asm__ volatile ("lwarx %1,0,%2\n\tdcbf 0,%2\n\tstwcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t) : "r"(res), "r"(st), "r"(v) : "cr0", "memory");
        break;
    case W_DCBST_W:
        __asm__ volatile ("lwarx %1,0,%2\n\tdcbst 0,%2\n\tstwcx. %4,0,%3\n\tmfcr %0"
                          : "=r"(cr), "=&r"(t) : "r"(res), "r"(st), "r"(v) : "cr0", "memory");
        break;
    case TWO:
        break;
    }
    return (cr >> 29) & 1;
}

/* The second thread: for each trial main raises flag_a to, run the cache
 * operation b_op names on the line, then answer on flag_b. */
static void toucher(u64 arg)
{
    unsigned int seen = 0;
    unsigned long line = (unsigned long)&buf[0];
    (void)arg;
    while (!b_stop) {
        unsigned int t = flag_a;
        if (t != seen) {
            seen = t;
            switch (b_op) {
            case 1:
                __asm__ volatile ("dcbst 0,%0" : : "r"(line) : "memory");
                break;
            case 2:
                __asm__ volatile ("dcbf 0,%0" : : "r"(line) : "memory");
                break;
            case 3:
                __asm__ volatile ("dcbtst 0,%0" : : "r"(line) : "memory");
                break;
            default:
                break;
            }
            __sync_synchronize();
            flag_b = t;
            __sync_synchronize();
        }
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* One two-thread trial: CR0[EQ], or 2 on timeout. */
static unsigned int two_thread(unsigned int t, unsigned long long v)
{
    unsigned long long tmp;
    unsigned int cr, arrived = 0;
    unsigned long long t0;
    unsigned long line = (unsigned long)&buf[0];
    __asm__ volatile ("lwarx %0,0,%1" : "=r"(tmp) : "r"(line) : "memory");
    flag_a = t;
    t0 = timebase();
    while (timebase() - t0 < SPIN_TICKS) {
        if (flag_b == t) {
            arrived = 1;
            break;
        }
    }
    __asm__ volatile ("stwcx. %2,0,%1\n\tmfcr %0"
                      : "=r"(cr) : "r"(line), "r"(v) : "cr0", "memory");
    if (!arrived)
        return 2;
    return (cr >> 29) & 1;
}

static void fill(void)
{
    unsigned int i;
    for (i = 0; i < 32; i++)
        buf[i] = SENT;
    __sync_synchronize();
}

/* Whether the target of a store-conditional of `width` bytes at `off`
 * holds the low bytes of `v`. */
static unsigned int holds(unsigned int off, unsigned int width, unsigned long long v)
{
    const volatile unsigned char *p = (const volatile unsigned char *)buf + off;
    unsigned long long got = 0;
    unsigned int i;
    for (i = 0; i < width; i++)
        got = (got << 8) | p[i];
    if (width == 4)
        v &= 0xFFFFFFFFull;
    return got == v;
}

static unsigned int store_width(enum kind kind)
{
    return (kind == WD || kind == DD) ? 8 : 4;
}

static struct thread_param_s {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
} param_b __attribute__((aligned(8)));

struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

static u64 make_param(const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    unsigned long r13;
    __asm__ volatile ("mr %0, 13" : "=r"(r13));
    param_b.opd_code = (unsigned int)desc->code;
    param_b.opd_toc = (unsigned int)desc->toc;
    param_b.entry_opd_ptr = (unsigned int)(unsigned long)&param_b.opd_code;
    param_b.tls = (unsigned int)r13;
    return (u64)(unsigned long)&param_b;
}

int main(void)
{
    unsigned long long tid, value;
    unsigned int rc, c, i, t = 0;

    memset((void *)result, 0, sizeof(result));
    result[2] = TRIALS;

    for (c = 0; c < 14; c++) {
        const struct pair *p = &CASE[c];
        unsigned long res = (unsigned long)&buf[0];
        unsigned long st = res + p->off;
        for (i = 0; i < TRIALS; i++) {
            unsigned long long v = 0x1122334455667700ull + (c << 8) + i;
            unsigned int eq;
            fill();
            eq = run_pair(p->kind, res, st, v);
            result[3 + 3 * c] += eq;
            result[4 + 3 * c] += holds(p->off, store_width(p->kind), v);
        }
    }

    flag_a = 0;
    flag_b = 0;
    b_stop = 0;
    b_op = 0;
    __sync_synchronize();
    rc = (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, (u64)(unsigned long)&tid,
                                    make_param((const void *)&toucher), 0, 0,
                                    CHILD_PRIO, STACK_SIZE, THREAD_JOINABLE, 0);
    if (rc != 0)
        return fail(1, (int)rc);
    rc = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0);
    if (rc != 0)
        return fail(2, (int)rc);

    for (c = 14; c < CASES; c++) {
        b_op = CASE[c].b_op;
        __sync_synchronize();
        for (i = 0; i < TRIALS; i++) {
            unsigned long long v = 0x1122334455667700ull + (c << 8) + i;
            unsigned int r;
            fill();
            r = two_thread(++t, v);
            if (r == 2) {
                result[5 + 3 * c]++;
                continue;
            }
            result[3 + 3 * c] += r;
            result[4 + 3 * c] += holds(0, 4, v);
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
