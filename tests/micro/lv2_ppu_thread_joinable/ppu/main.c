/* PPU program: what the JOINABLE flag of _sys_ppu_thread_create (52)
 * changes. A thread created with flags 0 (not joinable): what a join
 * answers while it runs and while it waits for its start, and whether
 * 44, 48 and 53 still find it after it exits. A thread created with
 * THREAD_JOINABLE, for contrast: 48 after it exits, then the join.
 *
 * Thread E (flags 0) runs spinner(): it sets e_reached, spins until main
 * sets e_release or SPIN_TICKS pass, sets e_done and exits with E_EXIT.
 * The timeout keeps a join that waits from waiting forever. Thread G
 * (JOINABLE) runs quick_g(): it sets g_done and exits with G_EXIT. Thread
 * H (flags 0) runs quick_h(); the probe joins it before it starts, then
 * starts it so that the process can exit.
 *
 * Result layout (64 bytes, big-endian u32 words; SKIP = 0x5C1F5C1F):
 *   w0: failed step (0 = none), w1: its return code
 *   w2:  52 E (flags 0)            w3:  53 E
 *   w4:  E's reached flag after up to WAIT_TICKS
 *   w5:  44(E, &v) while E spins   w6:  v low word after it
 *   w7:  E's done flag right after that join
 *   w8:  E's done flag after up to WAIT_TICKS once released
 *   w9:  44(E, &v) after E exited  w10: 48(E, &p) after E exited
 *   w11: 53(E) after E exited
 *   w12: G's done flag after up to WAIT_TICKS (52 G with THREAD_JOINABLE
 *        and 53 G must answer 0, or the probe fails at step 3 or 4)
 *   w13: 48(G, &p) after G exited  w14: 44(G, &v), v low word in the
 *        upper half when the join answers 0
 *   w15: 44(H, &v) before H starts (H is then started and joined)
 */

#include <string.h>

#include <sys/process.h>
#include <lv2/thread.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 64

#define SYS_PPU_THREAD_EXIT         41
#define SYS_PPU_THREAD_JOIN         44
#define SYS_PPU_THREAD_GET_PRIORITY 48

#define THREAD_JOINABLE 1

#define SKIP        0x5C1F5C1Fu
#define E_EXIT      0x0E
#define G_EXIT      0x6
#define WAIT_TICKS  400000ull
#define SPIN_TICKS  8000000ull
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

static volatile unsigned int e_reached;
static volatile unsigned int e_release;
static volatile unsigned int e_done;
static volatile unsigned int g_done;
static volatile unsigned int h_done;

static void spinner(u64 arg)
{
    unsigned long long start = timebase();
    (void)arg;
    e_reached = 1;
    __sync_synchronize();
    while (!e_release && timebase() - start < SPIN_TICKS) { }
    e_done = 1;
    __sync_synchronize();
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, E_EXIT);
    for (;;) { }
}

static void quick_g(u64 arg)
{
    (void)arg;
    g_done = 1;
    __sync_synchronize();
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, G_EXIT);
    for (;;) { }
}

static void quick_h(u64 arg)
{
    (void)arg;
    h_done = 1;
    __sync_synchronize();
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static struct thread_param param_e __attribute__((aligned(8)));
static struct thread_param param_g __attribute__((aligned(8)));
static struct thread_param param_h __attribute__((aligned(8)));
static unsigned long long g_value;
static int g_prio;

static unsigned int create(u64 tid_ptr, u64 param, u64 flags)
{
    return (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, tid_ptr, param, 0, 0,
                                      CHILD_PRIO, STACK_SIZE, flags, 0);
}

static unsigned int start(unsigned long long tid)
{
    return (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0);
}

/* Spin for WAIT_TICKS, long enough for an exiting thread to finish its
 * exit call. */
static void pause_ticks(void)
{
    unsigned long long t0 = timebase();
    while (timebase() - t0 < WAIT_TICKS) { }
}

static unsigned int wait_flag(volatile unsigned int *flag, unsigned long long ticks)
{
    unsigned long long t0 = timebase();
    while (timebase() - t0 < ticks) {
        if (*flag)
            return 1;
    }
    return *flag ? 1 : 0;
}

int main(void)
{
    unsigned long long e, g, h;
    u64 prio_ptr = (u64)(unsigned long)&g_prio;
    u64 value_ptr = (u64)(unsigned long)&g_value;
    unsigned int rc_create, rc_start, rc;

    memset((void *)result_buf, 0, sizeof(result_buf));
    e_reached = e_release = e_done = g_done = h_done = 0;

    /* Thread E: not joinable, joined while it runs. */
    result_buf[2] = create((u64)(unsigned long)&e, make_param(&param_e, (const void *)&spinner), 0);
    if (result_buf[2] != 0) return fail(1, (int)result_buf[2]);
    result_buf[3] = start(e);
    if (result_buf[3] != 0) return fail(2, (int)result_buf[3]);
    result_buf[4] = wait_flag(&e_reached, WAIT_TICKS);
    g_value = 0xFFFFFFFFFFFFFFFFull;
    result_buf[5] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, e, value_ptr);
    result_buf[6] = (unsigned int)g_value;
    result_buf[7] = e_done;
    e_release = 1;
    __sync_synchronize();
    result_buf[8] = wait_flag(&e_done, WAIT_TICKS);
    pause_ticks();
    g_value = 0xFFFFFFFFFFFFFFFFull;
    result_buf[9] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, e, value_ptr);
    result_buf[10] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, e, prio_ptr);
    result_buf[11] = start(e);

    /* Thread G: joinable, for contrast. */
    rc_create = create((u64)(unsigned long)&g, make_param(&param_g, (const void *)&quick_g),
                       THREAD_JOINABLE);
    if (rc_create != 0) return fail(3, (int)rc_create);
    rc_start = start(g);
    if (rc_start != 0) return fail(4, (int)rc_start);
    result_buf[12] = wait_flag(&g_done, WAIT_TICKS);
    pause_ticks();
    result_buf[13] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, g, prio_ptr);
    g_value = 0xFFFFFFFFFFFFFFFFull;
    rc = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, g, value_ptr);
    result_buf[14] = rc == 0 ? ((unsigned int)g_value << 16) : rc;

    /* Thread H: not joinable, joined before it starts, then started. */
    rc_create = create((u64)(unsigned long)&h, make_param(&param_h, (const void *)&quick_h), 0);
    if (rc_create != 0) return fail(5, (int)rc_create);
    result_buf[15] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, h, value_ptr);
    rc_start = start(h);
    if (rc_start != 0) return fail(6, (int)rc_start);
    (void)wait_flag(&h_done, WAIT_TICKS);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
