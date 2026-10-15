/* PPU program: the exact ceiling sys_ppu_thread_set_priority (47)
 * accepts for this process, between the 3072 lv2_ppu_thread_priority_bounds
 * accepts and the 3200 it refuses; and one create at a normal priority.
 *
 * 47: the caller sets its own priority to each value of SET_VALUES and
 * back to the priority it started with.
 * 52: one joinable thread per value of CREATE_VALUES, entry quitter()
 * (exit at once). A create that succeeds is read back with
 * sys_ppu_thread_get_priority (48), moved to CHILD_PRIO (a thread at a
 * priority outside the scheduler's range may never run, and then the
 * process cannot end), started, and joined.
 *
 * Result layout (192 bytes, big-endian u32 words; SKIP = 0x5C1F5C1F):
 *   w0: failed step (0 = none), w1: its return code
 *   w2-w13:  47(self, SET_VALUES[i])
 *   w14:     47(self, the starting priority)
 *   per CREATE_VALUES[i] at w(16 + 5i):
 *     +0 52 rc, +1 48 rc, +2 the priority 48 read, +3 47 to CHILD_PRIO,
 *     +4 the join rc after 53 (SKIP when the create failed)
 *   w21-w47: 0
 */

#include <string.h>

#include <sys/process.h>
#include <lv2/thread.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 192

#define SYS_PPU_THREAD_EXIT         41
#define SYS_PPU_THREAD_JOIN         44
#define SYS_PPU_THREAD_SET_PRIORITY 47
#define SYS_PPU_THREAD_GET_PRIORITY 48

#define THREAD_JOINABLE 1
#define SKIP            0x5C1F5C1Fu
#define STACK_SIZE      0x4000
#define CHILD_PRIO      1000

static const int SET_VALUES[12] = {
    3071, 3072, 3073, 3074, 3080, 3100, 3127, 3128, 3135, 3136, 3150, 3199
};
#define CREATE_COUNT 1
static const int CREATE_VALUES[CREATE_COUNT] = {
    1000
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

static void write_frame(const volatile void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)data, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)data, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result_buf[0] = step;
    result_buf[1] = (unsigned int)rc;
    write_frame(result_buf, RESULT_BYTES);
    return (int)step;
}

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

static void quitter(u64 arg)
{
    (void)arg;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static struct thread_param param_quit __attribute__((aligned(8)));
static int g_prio;
static unsigned long long g_value;

int main(void)
{
    sys_ppu_thread_t self = 0;
    const struct elfv1_opd *desc = (const struct elfv1_opd *)(const void *)&quitter;
    int start_prio;
    unsigned int i;
    int ret;

    memset((void *)result_buf, 0, sizeof(result_buf));

    ret = sysThreadGetId(&self);
    if (ret != 0) return fail(1, ret);
    ret = syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, self, (u64)(unsigned long)&g_prio);
    if (ret != 0) return fail(2, ret);
    start_prio = g_prio;

    for (i = 0; i < 12; i++) {
        result_buf[2 + i] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self,
                                                       (u64)(long long)SET_VALUES[i]);
        (void)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self, (u64)(long long)start_prio);
    }
    result_buf[14] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self,
                                                (u64)(long long)start_prio);

    param_quit.opd_code = (unsigned int)desc->code;
    param_quit.opd_toc = (unsigned int)desc->toc;
    param_quit.entry_opd_ptr = (unsigned int)(unsigned long)&param_quit.opd_code;
    param_quit.tls = self_tls();

    for (i = 0; i < CREATE_COUNT; i++) {
        volatile unsigned int *w = &result_buf[16 + 5 * i];
        unsigned long long tid = 0;
        unsigned int rc = (unsigned int)syscall8_s32(
            CGOV_SYS_PPU_THREAD_CREATE, (u64)(unsigned long)&tid,
            (u64)(unsigned long)&param_quit, 0, 0, (u64)(long long)CREATE_VALUES[i],
            STACK_SIZE, THREAD_JOINABLE, 0);
        w[0] = rc;
        if (rc != 0) {
            w[1] = SKIP;
            w[2] = SKIP;
            w[3] = SKIP;
            w[4] = SKIP;
            continue;
        }
        g_prio = -7;
        w[1] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, tid,
                                          (u64)(unsigned long)&g_prio);
        w[2] = (unsigned int)g_prio;
        w[3] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, tid, CHILD_PRIO);
        if (w[3] != 0)
            return fail(3, (int)w[3]);
        if (syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0) != 0)
            return fail(4, (int)i);
        w[4] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (u64)(unsigned long)&g_value);
    }

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
