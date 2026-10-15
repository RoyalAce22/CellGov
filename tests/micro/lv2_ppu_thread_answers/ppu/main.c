/* PPU program: what the PPU thread calls answer on the console.
 * sys_ppu_thread_get_priority (48) and sys_ppu_thread_set_priority (47)
 * against an unknown id and the caller; the check order of
 * _sys_ppu_thread_create (52); sys_ppu_thread_join (44) with a NULL
 * vptr; whether a thread created by 52 runs before
 * sys_ppu_thread_start (53); what a second 53 answers; and which word
 * of the thread-init block the new thread takes as r13.
 *
 * Thread A runs sleeper(): it sleeps SLEEP_USEC and exits with
 * A_EXIT. Thread B runs marker(): it stores its r13 and a reached flag
 * and exits with B_EXIT. A create that should fail but succeeds gets
 * quitter() (exit at once) and is started, so it ends; a create whose
 * block names no entry is never started. No join names a thread that
 * has not run.
 *
 * Result layout (192 bytes, big-endian u32 words; SKIP = 0x5C1F5C1F):
 *   w0: failed step (0 = none), w1: its return code
 *   w2:  48(NO_SUCH_ID, NULL)      w3:  48(NO_SUCH_ID, &p)
 *   w4:  48(self, NULL)            w5:  48(self, &p), w6: p
 *   w7:  47(NO_SUCH_ID, 5000)      w8:  47(NO_SUCH_ID, 100)
 *   w9:  47(self, 5000)            w10: 47(self, -1)
 *   w11: 47(self, -512)            w12: 47(self, -513)
 *   w13: 47(self, 3071)            w14: 47(self, 3072)
 *   w15: 47(self, the priority w6 read)
 *   w16: 52(param NULL, prio 9999)
 *   w17: 52(entry 0, prio 9999)
 *   w18: 52(flags 3)
 *   w19: 52(flags 3, prio 9999)
 *   w20: 52(flags 3, thread_id NULL)
 *   w21: 52(prio 9999)
 *   w22: 52(prio -1)
 *   w23: 52(thread_id NULL)
 *   w24: 52 A, w25: 53 A, w26: 53 A again while it sleeps
 *   w27: 44(A, NULL), w28: timebase ticks that join took
 *   w29: 44(A, &v), w30: v low word
 *   w31: 52 B, w32: B's reached flag after WAIT_TICKS with no 53
 *   w33: 53 B, w34: B's reached flag after up to WAIT_TICKS more
 *   w35: B's r13 low word, w36: the tls word of B's block
 *   w37: 53 B again after it ran
 *   w38: 44(B, &v), w39: v low word
 *   w40: 44(NO_SUCH_ID, &v)        w41: 44(self, &v)
 *   w42: the extra threads the failure-path creates made and started
 *   w43: 47 to CHILD_PRIO on the thread w21 made (SKIP when none)
 *   w44: 47 to CHILD_PRIO on the thread w22 made (SKIP when none)
 *   w45-w47: 0
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
#define SYS_TIMER_USLEEP            141

#define THREAD_JOINABLE  1
#define THREAD_INTERRUPT 2

#define NO_SUCH_ID  0xDEADBEEFu
#define SKIP        0x5C1F5C1Fu
#define A_EXIT      0x1234
#define B_EXIT      0x55
#define B_TLS       0x1BADBEE0u
#define SLEEP_USEC  10000
#define WAIT_TICKS  400000ull
#define STACK_SIZE  0x4000
#define CHILD_PRIO  1000

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

static u64 make_param(struct thread_param *p, const void *fn, unsigned int tls)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = tls;
    return (u64)(unsigned long)p;
}

static inline unsigned long long timebase(void)
{
    unsigned long long tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static volatile unsigned int b_reached;
static volatile unsigned int b_r13;

static void sleeper(u64 arg)
{
    (void)arg;
    (void)syscall1_s32(SYS_TIMER_USLEEP, SLEEP_USEC);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, A_EXIT);
    for (;;) { }
}

static void marker(u64 arg)
{
    unsigned long r13;
    (void)arg;
    __asm__ volatile ("mr %0, 13" : "=r"(r13));
    b_r13 = (unsigned int)r13;
    __sync_synchronize();
    b_reached = 1;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, B_EXIT);
    for (;;) { }
}

static void quitter(u64 arg)
{
    (void)arg;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static struct thread_param param_a __attribute__((aligned(8)));
static struct thread_param param_b __attribute__((aligned(8)));
static struct thread_param param_quit __attribute__((aligned(8)));
static struct thread_param param_no_entry __attribute__((aligned(8)));
static unsigned long long g_tid;
static unsigned long long g_value;
static int g_prio;
static unsigned int extra_threads;

static unsigned int create(u64 tid_ptr, u64 param, u64 prio, u64 flags)
{
    return (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, tid_ptr, param, 0, 0,
                                      prio, STACK_SIZE, flags, 0);
}

/* A create that should be refused, with quitter() as its entry: if
 * the console takes it, move the thread to CHILD_PRIO (a thread left at
 * a priority outside the scheduler's range may never run, and then the
 * process cannot end) and start it so that it ends. The 47's answer goes
 * to result word `fix_word`. */
static unsigned int refused_create(u64 tid_ptr, u64 prio, u64 flags, unsigned int fix_word)
{
    unsigned long long tid = 0;
    u64 ptr = tid_ptr ? (u64)(unsigned long)&tid : 0;
    unsigned int rc = create(ptr, make_param(&param_quit, (const void *)&quitter, self_tls()),
                             prio, flags);
    if (fix_word != 0)
        result_buf[fix_word] = SKIP;
    if (rc == 0 && ptr) {
        extra_threads += 1;
        if (fix_word != 0)
            result_buf[fix_word] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, tid,
                                                              CHILD_PRIO);
        (void)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0);
    }
    return rc;
}

static int wait_reached(unsigned long long ticks)
{
    unsigned long long start = timebase();
    while (timebase() - start < ticks) {
        if (b_reached)
            return 1;
    }
    return b_reached ? 1 : 0;
}

int main(void)
{
    sys_ppu_thread_t self = 0;
    unsigned long long a, b, t0;
    u64 prio_ptr = (u64)(unsigned long)&g_prio;
    u64 value_ptr = (u64)(unsigned long)&g_value;
    int ret;

    memset((void *)result_buf, 0, sizeof(result_buf));

    ret = sysThreadGetId(&self);
    if (ret != 0) return fail(1, ret);

    /* Priorities. */
    result_buf[2] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, NO_SUCH_ID, 0);
    result_buf[3] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, NO_SUCH_ID, prio_ptr);
    result_buf[4] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, self, 0);
    g_prio = -7;
    result_buf[5] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, self, prio_ptr);
    result_buf[6] = (unsigned int)g_prio;
    if (result_buf[5] != 0) return fail(2, (int)result_buf[5]);
    result_buf[7] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, NO_SUCH_ID, 5000);
    result_buf[8] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, NO_SUCH_ID, 100);
    result_buf[9] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self, 5000);
    result_buf[10] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self, (u64)(long long)-1);
    result_buf[11] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self, (u64)(long long)-512);
    result_buf[12] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self, (u64)(long long)-513);
    result_buf[13] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self, 3071);
    result_buf[14] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self, 3072);
    result_buf[15] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_SET_PRIORITY, self, (u64)(long long)g_prio);

    /* Create check order. A block naming no entry is never started. */
    result_buf[16] = create((u64)(unsigned long)&g_tid, 0, 9999, 0);
    param_no_entry.entry_opd_ptr = 0;
    param_no_entry.tls = self_tls();
    result_buf[17] = create((u64)(unsigned long)&g_tid, (u64)(unsigned long)&param_no_entry, 9999, 0);
    result_buf[18] = refused_create(1, CHILD_PRIO, THREAD_JOINABLE | THREAD_INTERRUPT, 0);
    result_buf[19] = refused_create(1, 9999, THREAD_JOINABLE | THREAD_INTERRUPT, 0);
    result_buf[20] = refused_create(0, CHILD_PRIO, THREAD_JOINABLE | THREAD_INTERRUPT, 0);
    result_buf[21] = refused_create(1, 9999, 0, 43);
    result_buf[22] = refused_create(1, (u64)(long long)-1, 0, 44);
    result_buf[23] = refused_create(0, CHILD_PRIO, 0, 0);

    /* Thread A: a join with a NULL vptr, then one with a pointer. */
    result_buf[24] = create((u64)(unsigned long)&a,
                            make_param(&param_a, (const void *)&sleeper, self_tls()),
                            CHILD_PRIO, THREAD_JOINABLE);
    if (result_buf[24] != 0) return fail(3, (int)result_buf[24]);
    result_buf[25] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, a, 0);
    if (result_buf[25] != 0) return fail(4, (int)result_buf[25]);
    result_buf[26] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, a, 0);
    t0 = timebase();
    result_buf[27] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, a, 0);
    result_buf[28] = (unsigned int)(timebase() - t0);
    g_value = 0xFFFFFFFFFFFFFFFFull;
    if (result_buf[27] == 0 || result_buf[27] == 0x8001000Du) {
        result_buf[29] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, a, value_ptr);
        result_buf[30] = (unsigned int)g_value;
    } else {
        result_buf[29] = SKIP;
        result_buf[30] = SKIP;
    }

    /* Thread B: does it run before 53, and which r13 does it take. */
    b_reached = 0;
    b_r13 = 0;
    result_buf[31] = create((u64)(unsigned long)&b,
                            make_param(&param_b, (const void *)&marker, B_TLS),
                            CHILD_PRIO, THREAD_JOINABLE);
    if (result_buf[31] != 0) return fail(5, (int)result_buf[31]);
    result_buf[32] = (unsigned int)wait_reached(WAIT_TICKS);
    result_buf[33] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, b, 0);
    result_buf[34] = (unsigned int)wait_reached(WAIT_TICKS);
    result_buf[35] = b_r13;
    result_buf[36] = param_b.tls;
    result_buf[37] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, b, 0);
    g_value = 0xFFFFFFFFFFFFFFFFull;
    if (result_buf[34] == 1) {
        result_buf[38] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, b, value_ptr);
        result_buf[39] = (unsigned int)g_value;
    } else {
        result_buf[38] = SKIP;
        result_buf[39] = SKIP;
    }

    result_buf[40] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, NO_SUCH_ID, value_ptr);
    result_buf[41] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, self, value_ptr);
    result_buf[42] = extra_threads;

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
