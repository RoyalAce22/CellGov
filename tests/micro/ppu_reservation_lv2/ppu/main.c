/* PPU probe: whether LV2 keeps a reservation across a system call and
 * across a context switch.
 *
 * The reserved line is the 128-byte-aligned start of a 256-byte buffer.
 * Each trial: lwarx on line + 0, then the case's step, then stwcx. on
 * line + 0, recording CR0[EQ]. TRIALS trials per case.
 *   0  control: nothing between the two
 *   1  sys_time_get_current_time (145), its outputs on another line
 *   2  sys_time_get_current_time (145), its outputs at line + 64 and
 *      line + 72 (inside the reserved granule)
 *   3  sys_ppu_thread_yield (43) while a second thread of equal priority
 *      is runnable; it counts its loops on its own line and touches
 *      nothing near the reserved one
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  TRIALS
 *   3 + 3c, case c: CR0[EQ] set, syscall returned 0, trials in which the
 *               second thread's loop count moved (case 3 only)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 4
#define WORDS (3 + 3 * CASES)
#define TRIALS 32

#define SYS_PPU_THREAD_EXIT 41
#define SYS_PPU_THREAD_YIELD 43
#define SYS_PPU_THREAD_JOIN 44
#define SYS_TIME_GET_CURRENT_TIME 145
#define THREAD_JOINABLE 1
#define PRIO 1001
#define STACK_SIZE 0x4000

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long buf[32] __attribute__((aligned(128)));
static volatile unsigned long long elsewhere[2] __attribute__((aligned(128)));
static volatile unsigned int loops __attribute__((aligned(128)));
static volatile unsigned int b_stop __attribute__((aligned(128)));

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

static inline void reserve(void)
{
    unsigned int t;
    __asm__ volatile ("lwarx %0,0,%1" : "=r"(t) : "r"(&buf[0]) : "memory");
}

static inline unsigned int store_conditional(unsigned int v)
{
    unsigned int cr;
    __asm__ volatile ("stwcx. %2,0,%1\n\tmfcr %0"
                      : "=r"(cr) : "r"(&buf[0]), "r"(v) : "cr0", "memory");
    return (cr >> 29) & 1;
}

/* The second thread: yields in a loop and counts the loops on its own
 * line until main sets b_stop. */
static void spinner(u64 arg)
{
    (void)arg;
    while (!b_stop) {
        loops = loops + 1;
        (void)syscall0_s32(SYS_PPU_THREAD_YIELD);
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

static struct {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
} param_b __attribute__((aligned(8)));

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

static void run_case(unsigned int c)
{
    unsigned int i;
    for (i = 0; i < TRIALS; i++) {
        unsigned int before = loops;
        s32 rc = 0;
        reserve();
        switch (c) {
        case 1:
            rc = syscall2_s32(SYS_TIME_GET_CURRENT_TIME,
                              (u64)(unsigned long)&elsewhere[0],
                              (u64)(unsigned long)&elsewhere[1]);
            break;
        case 2:
            rc = syscall2_s32(SYS_TIME_GET_CURRENT_TIME,
                              (u64)(unsigned long)&buf[8],
                              (u64)(unsigned long)&buf[9]);
            break;
        case 3:
            rc = syscall0_s32(SYS_PPU_THREAD_YIELD);
            break;
        default:
            break;
        }
        result[3 + 3 * c] += store_conditional(0x5A5A0000u + i);
        result[4 + 3 * c] += rc == 0;
        if (c == 3)
            result[5 + 3 * c] += loops != before;
    }
}

int main(void)
{
    unsigned long long tid, value;
    unsigned int c;
    s32 rc;

    memset((void *)result, 0, sizeof(result));
    result[2] = TRIALS;

    for (c = 0; c < 3; c++)
        run_case(c);

    loops = 0;
    b_stop = 0;
    __sync_synchronize();
    rc = cgov_ppu_thread_create(&tid, make_param((const void *)&spinner), 0, 0,
                                PRIO, STACK_SIZE, THREAD_JOINABLE, 0);
    if (rc != 0)
        return fail(1, rc);
    run_case(3);

    b_stop = 1;
    __sync_synchronize();
    rc = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (u64)(unsigned long)&value);
    if (rc != 0)
        return fail(2, rc);

    write_frame();
    return 0;
}
