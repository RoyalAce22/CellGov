/* PPU program: the sys_ppu_thread_start (53) answers lv2_ppu_thread_answers
 * left open. 53 against an unknown id and against the caller; 53 on a
 * thread a join already collected; and a thread _sys_ppu_thread_create
 * (52) made and has not started: whether it runs, and what 48 and 47
 * answer on it.
 *
 * Thread C runs marker(): it stores a reached flag and exits with C_EXIT.
 * Thread D runs marker_d(): it stores its own flag and exits with D_EXIT.
 * D is started only after w15 records that it has not run. A run that
 * left D unstarted wrote this frame and then did not return to the XMB:
 * a process does not finish exiting while it holds a thread that never
 * started.
 *
 * Result layout (80 bytes, big-endian u32 words; SKIP = 0x5C1F5C1F):
 *   w0: failed step (0 = none), w1: its return code
 *   w2:  53(NO_SUCH_ID)            w3:  53(self)
 *   w4:  52 C                      w5:  53 C
 *   w6:  C's reached flag after up to WAIT_TICKS
 *   w7:  44(C, &v), w8: v low word
 *   w9:  53 C after the join
 *   w10: 52 D                      w11: 48(D, &p), w12: p
 *   w13: 47(D, D_PRIO)
 *   w14: the p a 48(D, &p) reads after that (its code when not CELL_OK)
 *   w15: D's reached flag after WAIT_TICKS with no 53
 *   w16: 53 D                      w17: D's reached flag after up to
 *        WAIT_TICKS more
 *   w18: 44(D, &v), w19: v low word
 */

#include <string.h>

#include <sys/process.h>
#include <lv2/thread.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 80

#define SYS_PPU_THREAD_EXIT         41
#define SYS_PPU_THREAD_JOIN         44
#define SYS_PPU_THREAD_SET_PRIORITY 47
#define SYS_PPU_THREAD_GET_PRIORITY 48

#define THREAD_JOINABLE 1

#define NO_SUCH_ID  0xDEADBEEFu
#define SKIP        0x5C1F5C1Fu
#define C_EXIT      0x77
#define D_EXIT      0x99
#define WAIT_TICKS  400000ull
#define STACK_SIZE  0x4000
#define CHILD_PRIO  1000
#define D_PRIO      1500

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

static inline unsigned long long timebase(void)
{
    unsigned long long tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static volatile unsigned int c_reached;
static volatile unsigned int d_reached;

static void marker(u64 arg)
{
    (void)arg;
    __sync_synchronize();
    c_reached = 1;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, C_EXIT);
    for (;;) { }
}

static void marker_d(u64 arg)
{
    (void)arg;
    __sync_synchronize();
    d_reached = 1;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, D_EXIT);
    for (;;) { }
}

static struct thread_param param_c __attribute__((aligned(8)));
static struct thread_param param_d __attribute__((aligned(8)));
static unsigned long long g_value;
static int g_prio;

static unsigned int create(u64 tid_ptr, u64 param)
{
    return (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, tid_ptr, param, 0, 0,
                                      CHILD_PRIO, STACK_SIZE, THREAD_JOINABLE, 0);
}

static unsigned int wait_flag(volatile unsigned int *flag, unsigned long long ticks)
{
    unsigned long long start = timebase();
    while (timebase() - start < ticks) {
        if (*flag)
            return 1;
    }
    return *flag ? 1 : 0;
}

int main(void)
{
    sys_ppu_thread_t self = 0;
    unsigned long long c, d;
    u64 prio_ptr = (u64)(unsigned long)&g_prio;
    u64 value_ptr = (u64)(unsigned long)&g_value;
    int ret;

    memset((void *)result_buf, 0, sizeof(result_buf));
    c_reached = 0;
    d_reached = 0;

    ret = sysThreadGetId(&self);
    if (ret != 0) return fail(1, ret);

    result_buf[2] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, NO_SUCH_ID, 0);
    result_buf[3] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, self, 0);

    /* Thread C: started, joined, then started again. */
    result_buf[4] = create((u64)(unsigned long)&c, make_param(&param_c, (const void *)&marker));
    if (result_buf[4] != 0) return fail(2, (int)result_buf[4]);
    result_buf[5] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, c, 0);
    if (result_buf[5] != 0) return fail(3, (int)result_buf[5]);
    result_buf[6] = wait_flag(&c_reached, WAIT_TICKS);
    g_value = 0xFFFFFFFFFFFFFFFFull;
    if (result_buf[6] == 1) {
        result_buf[7] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, c, value_ptr);
        result_buf[8] = (unsigned int)g_value;
        result_buf[9] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, c, 0);
    } else {
        result_buf[7] = SKIP;
        result_buf[8] = SKIP;
        result_buf[9] = SKIP;
    }

    /* Thread D: created and never started. */
    result_buf[10] = create((u64)(unsigned long)&d, make_param(&param_d, (const void *)&marker_d));
    if (result_buf[10] != 0) return fail(4, (int)result_buf[10]);
    g_prio = -7;
    result_buf[11] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, d, prio_ptr);
    result_buf[12] = (unsigned int)g_prio;
    result_buf[13] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, d, D_PRIO);
    g_prio = -7;
    result_buf[14] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, d, prio_ptr);
    result_buf[14] = result_buf[14] == 0 ? (unsigned int)g_prio : result_buf[14];
    result_buf[15] = wait_flag(&d_reached, WAIT_TICKS);

    /* Start D so that the process can exit. */
    result_buf[16] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, d, 0);
    if (result_buf[16] != 0) return fail(5, (int)result_buf[16]);
    result_buf[17] = wait_flag(&d_reached, WAIT_TICKS);
    g_value = 0xFFFFFFFFFFFFFFFFull;
    if (result_buf[17] == 1) {
        result_buf[18] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, d, value_ptr);
        result_buf[19] = (unsigned int)g_value;
    } else {
        result_buf[18] = SKIP;
        result_buf[19] = SKIP;
    }

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
