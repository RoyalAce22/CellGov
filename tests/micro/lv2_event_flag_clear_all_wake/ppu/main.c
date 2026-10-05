/* PPU probe: an event flag set that wakes a waiter parked with
 * CLEAR_ALL, and what a second waiter parked behind it sees.
 *
 * Steps:
 *   1. sc 82 creates a flag (initial bits 0, any number of waiters).
 *   2. Waiter A starts and parks: sc 85 bits 1, mode OR|CLEAR_ALL.
 *      Main sleeps 20 ms. Waiter B starts and parks: sc 85 bits 2,
 *      mode OR. Both wait with a 200 ms timeout; both run at a higher
 *      priority than main. Main sleeps 20 ms more.
 *   3. Main sets 0b11 (sc 87), sleeps 20 ms, and reads the bits
 *      (sc 139). It records which waiters had returned by then.
 *   4. Main joins both waiters, then destroys the flag.
 *
 * Payload layout (14 big-endian u32 words, 56 bytes):
 *   0  step that failed (0 = none; 1 = create, 2 = a thread create,
 *      3 = a join)
 *   1  that step's return code
 *   2  set rc
 *   3,4  bits 20 ms after the set (high, low)
 *   5  1 when A had returned 20 ms after the set
 *   6  1 when B had returned 20 ms after the set
 *   7  A's return code      8,9   A's result (high, low)
 *   10 B's return code      11,12 B's result (high, low)
 *   13 destroy rc
 */

#include <string.h>

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT      41
#define SYS_PPU_THREAD_JOIN      44
#define SYS_EVENT_FLAG_CREATE    82
#define SYS_EVENT_FLAG_DESTROY   83
#define SYS_EVENT_FLAG_WAIT      85
#define SYS_EVENT_FLAG_SET       87
#define SYS_EVENT_FLAG_GET       139
#define SYS_TIMER_USLEEP         141

#define MODE_OR         0x02
#define MODE_CLEAR_ALL  0x20

#define TIMEOUT_US 200000u
#define SETTLE_US  20000u

/* FIFO, private, key 0, flags 0, any number of waiters. */
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x20000, 0, 0 };

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

/* Repack the 24-byte ELFv1 descriptor into the kernel's 8-byte form,
 * as ppu_event_flag_wakeall does. */
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

static struct cg_thread_param param_a __attribute__((aligned(8)));
static struct cg_thread_param param_b __attribute__((aligned(8)));

static unsigned int result[14] __attribute__((aligned(128)));
static unsigned int flag_id __attribute__((aligned(8)));
static volatile unsigned int done_a, done_b, rc_a, rc_b;
static volatile unsigned long long out_a __attribute__((aligned(8)));
static volatile unsigned long long out_b __attribute__((aligned(8)));
static volatile unsigned long long bits_word __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(void)
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
    sysTtyWrite(0, result, len, &written);
    CGOV_OUT_FILE_WRITE(result, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result[0] = step;
    result[1] = (unsigned int)rc;
    write_result();
    return (int)step;
}

#define EA(p) ((unsigned long)(const void *)(p))

static void waiter_a(void *arg)
{
    (void)arg;
    out_a = 0xFFFFFFFFFFFFFFFFULL;
    rc_a = (unsigned int)syscall5_s32(SYS_EVENT_FLAG_WAIT, flag_id, 1,
                                      MODE_OR | MODE_CLEAR_ALL, EA(&out_a), TIMEOUT_US);
    done_a = 1;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static void waiter_b(void *arg)
{
    (void)arg;
    out_b = 0xFFFFFFFFFFFFFFFFULL;
    rc_b = (unsigned int)syscall5_s32(SYS_EVENT_FLAG_WAIT, flag_id, 2, MODE_OR,
                                      EA(&out_b), TIMEOUT_US);
    done_b = 1;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

int main(void)
{
    unsigned long long tid_a = 0, tid_b = 0, retval = 0;
    unsigned long long v;
    s32 rc;

    memset(result, 0, sizeof(result));
    rc = syscall3_s32(SYS_EVENT_FLAG_CREATE, EA(&flag_id), EA(flag_attr), 0);
    if (rc != 0)
        return fail(1, rc);

    rc = cgov_ppu_thread_create(&tid_a, make_thread_param(&param_a, (const void *)&waiter_a),
                                0, 0, 1000, 0x4000, THREAD_JOINABLE, 0);
    if (rc != 0)
        return fail(2, rc);
    (void)syscall1_s32(SYS_TIMER_USLEEP, SETTLE_US);
    rc = cgov_ppu_thread_create(&tid_b, make_thread_param(&param_b, (const void *)&waiter_b),
                                0, 0, 1000, 0x4000, THREAD_JOINABLE, 0);
    if (rc != 0)
        return fail(2, rc);
    (void)syscall1_s32(SYS_TIMER_USLEEP, SETTLE_US);

    result[2] = (unsigned int)syscall2_s32(SYS_EVENT_FLAG_SET, flag_id, 3);
    (void)syscall1_s32(SYS_TIMER_USLEEP, SETTLE_US);
    bits_word = 0xFFFFFFFFFFFFFFFFULL;
    (void)syscall2_s32(SYS_EVENT_FLAG_GET, flag_id, EA(&bits_word));
    v = bits_word;
    result[3] = (unsigned int)(v >> 32);
    result[4] = (unsigned int)v;
    result[5] = done_a;
    result[6] = done_b;

    if (syscall2_s32(SYS_PPU_THREAD_JOIN, tid_a, EA(&retval)) != 0)
        return fail(3, 0);
    if (syscall2_s32(SYS_PPU_THREAD_JOIN, tid_b, EA(&retval)) != 0)
        return fail(3, 1);
    result[7] = rc_a;
    v = out_a;
    result[8] = (unsigned int)(v >> 32);
    result[9] = (unsigned int)v;
    result[10] = rc_b;
    v = out_b;
    result[11] = (unsigned int)(v >> 32);
    result[12] = (unsigned int)v;
    result[13] = (unsigned int)syscall1_s32(SYS_EVENT_FLAG_DESTROY, flag_id);

    write_result();
    return 0;
}
