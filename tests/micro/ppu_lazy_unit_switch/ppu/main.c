/* PPU program: whether the kernel resumes a thread with the
 * floating-point and vector units turned off in its MSR, so that the
 * thread's first floating-point or vector instruction after a switch
 * takes an unavailable interrupt.
 *
 * A user program cannot read its MSR. It can time one instruction with
 * the time base. Each round makes the kernel switch the thread out,
 * then times six instructions in a fixed order: an integer add twice,
 * a floating-point add twice and a vector add twice. Each second
 * instruction of a pair is the warm control: the unit is on by then.
 * The integer pair is the control for what the switch itself costs
 * (cache and translation misses). An interrupt on the first
 * floating-point or vector add shows as a first delta well above the
 * integer first delta and the warm delta.
 *
 * Section A switches with sys_timer_usleep (141) of SLEEP_US. Section B
 * creates a joinable helper thread that runs floating-point and vector
 * adds and exits, and joins it (44), so that another thread used both
 * units on the hardware thread before the timed instructions.
 *
 * Calls: 141, _sys_ppu_thread_create (52), sys_ppu_thread_start (53),
 * sys_ppu_thread_join (44) and sys_ppu_thread_exit (41) from the helper.
 * Every helper is joined, so the probe exits with no thread it made.
 *
 * Result layout (big-endian u32 words, 1184 bytes):
 *   w0: failed step (0 = none), w1: its return code
 *   w2: helper check (HELPER_DONE once every helper ran)
 *   w3: rounds in section A (REPS_A), w4: rounds in section B (REPS_B)
 *   w5..w7: zero
 *   w8 + 6*r + k, r < REPS_A: section A round r, time-base ticks of
 *       k = 0 integer first, 1 integer warm, 2 floating-point first,
 *       3 floating-point warm, 4 vector first, 5 vector warm
 *   w200 + 6*r + k, r < REPS_B: section B round r, the same six deltas
 *   w296: zero
 */

#include <string.h>

#include <sys/process.h>
#include <lv2/thread.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS        296
#define RESULT_BYTES (WORDS * 4)

#define SYS_PPU_THREAD_EXIT 41
#define SYS_PPU_THREAD_JOIN 44
#define SYS_TIMER_USLEEP    141

#define THREAD_JOINABLE 1
#define STACK_SIZE      0x4000
#define CHILD_PRIO      1000

#define REPS_A      32
#define REPS_B      16
#define BASE_A      8
#define BASE_B      200
#define SLEEP_US    100
#define HELPER_LOOP 4096
#define HELPER_DONE 0x600Du

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[WORDS] __attribute__((aligned(128)));

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

/* Ticks across one instruction. The operand registers are not
 * initialized first: loading them would itself use the unit. The
 * floating-point exception enables are clear, so fadd on any bits
 * raises no interrupt. vaddubm v1,v2,v3 is spelled as its word. */
static inline unsigned int time_int(void)
{
    unsigned long long a, b, x = 1;
    __asm__ volatile ("mftb %0\n\tadd %2,%2,%2\n\tmftb %1"
                      : "=&r"(a), "=&r"(b), "+r"(x) :: "memory");
    return (unsigned int)(b - a);
}

static inline unsigned int time_fp(void)
{
    unsigned long long a, b;
    double x;
    __asm__ volatile ("mftb %0\n\tfadd %2,%2,%2\n\tmftb %1"
                      : "=&r"(a), "=&r"(b), "=&f"(x) :: "memory");
    return (unsigned int)(b - a);
}

static inline unsigned int time_vec(void)
{
    unsigned long long a, b;
    __asm__ volatile ("mftb %0\n\t.long 0x10221800\n\tmftb %1"
                      : "=&r"(a), "=&r"(b) :: "memory", "v1");
    return (unsigned int)(b - a);
}

static void __attribute__((noinline)) round_deltas(unsigned int base)
{
    unsigned int d[6];
    d[0] = time_int();
    d[1] = time_int();
    d[2] = time_fp();
    d[3] = time_fp();
    d[4] = time_vec();
    d[5] = time_vec();
    for (unsigned int k = 0; k < 6; k++)
        result_buf[base + k] = d[k];
}

static volatile unsigned int helper_runs;

static void helper(u64 arg)
{
    (void)arg;
    for (unsigned int i = 0; i < HELPER_LOOP; i++) {
        double x;
        __asm__ volatile ("fadd %0,%0,%0\n\t.long 0x10221800"
                          : "=&f"(x) :: "v1");
    }
    helper_runs++;
    __sync_synchronize();
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static struct thread_param param_h __attribute__((aligned(8)));
static unsigned long long exit_value;

int main(void)
{
    unsigned long long tid;
    unsigned int rc;

    memset((void *)result_buf, 0, sizeof(result_buf));
    helper_runs = 0;
    result_buf[3] = REPS_A;
    result_buf[4] = REPS_B;

    /* Use every unit once, so no first use in the process is timed. */
    round_deltas(BASE_A);

    for (unsigned int r = 0; r < REPS_A; r++) {
        rc = (unsigned int)syscall1_s32(SYS_TIMER_USLEEP, SLEEP_US);
        if (rc != 0) return fail(1, (int)rc);
        round_deltas(BASE_A + 6 * r);
    }

    for (unsigned int r = 0; r < REPS_B; r++) {
        rc = (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, (u64)(unsigned long)&tid,
                                        make_param(&param_h, (const void *)&helper), 0, 0,
                                        CHILD_PRIO, STACK_SIZE, THREAD_JOINABLE, 0);
        if (rc != 0) return fail(2, (int)rc);
        rc = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0);
        if (rc != 0) return fail(3, (int)rc);
        rc = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid,
                                        (u64)(unsigned long)&exit_value);
        if (rc != 0) return fail(4, (int)rc);
        round_deltas(BASE_B + 6 * r);
    }

    result_buf[2] = helper_runs == REPS_B ? HELPER_DONE : helper_runs;
    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
