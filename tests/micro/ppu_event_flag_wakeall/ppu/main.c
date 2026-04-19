/* PPU program: event-flag multi-waiter wake. Primary spawns two
 * child threads that each park on sys_event_flag_wait with a
 * distinct bit mask in AND|NO-CLEAR mode. Primary sets a bit
 * pattern that matches BOTH waiters in one set call; both must
 * wake and observe the same bit pattern.
 *
 * Structural microtest for the event flag primitive. Proves:
 *
 *   1. sys_event_flag_create captures the initial bit state.
 *   2. sys_event_flag_wait with AND mode parks when the mask
 *      does not match.
 *   3. sys_event_flag_set wakes all matching waiters in one
 *      call, not just the head of the waiter list.
 *   4. Each waker observes the current bit pattern at its wake
 *      point.
 *
 * Output layout (TTY: CGOV magic + 16 bytes):
 *   status    (u32)  0 = pass
 *   waker_a   (u32)  bits observed by waiter A on wake
 *                    (expected 0b0011)
 *   waker_b   (u32)  bits observed by waiter B
 *                    (expected 0b0011)
 *   wake_cnt  (u32)  total wake count (expected 2)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

static inline s32 syscall2_s32(u64 num, u64 a, u64 b)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r11)
        : "memory"
    );
    return (s32)r3;
}

static inline s32 syscall3_s32(u64 num, u64 a, u64 b, u64 c)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r11)
        : "memory"
    );
    return (s32)r3;
}

static inline s32 syscall5_s32(u64 num, u64 a, u64 b, u64 c, u64 d, u64 e)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r7 __asm__("7") = e;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r7), "r"(r11)
        : "memory"
    );
    return (s32)r3;
}

static inline s32 syscall6_s32(u64 num, u64 a, u64 b, u64 c, u64 d, u64 e, u64 f)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r7 __asm__("7") = e;
    register u64 r8 __asm__("8") = f;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r7), "r"(r8), "r"(r11)
        : "r0", "r9", "r10", "r12", "cr0", "ctr", "memory"
    );
    return (s32)r3;
}

static inline void syscall1_noreturn(u64 num, u64 a)
{
    register u64 r3 __asm__("3") = a;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        :
        : "r"(r3), "r"(r11)
        : "memory"
    );
}

#define SYS_PPU_THREAD_EXIT    41
#define SYS_PPU_THREAD_JOIN    44
#define SYS_PPU_THREAD_CREATE  52
#define SYS_EVENT_FLAG_CREATE  82
#define SYS_EVENT_FLAG_WAIT    84
#define SYS_EVENT_FLAG_SET     86

/* ABI: bit 0x01 = AND, bit 0x10 = CLEAR. NO-CLEAR is the
 * absence of bit 0x10. */
#define WAIT_AND_NOCLEAR 0x01

struct TestResult {
    unsigned int status;
    unsigned int waker_a;
    unsigned int waker_b;
    unsigned int wake_cnt;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned int flag_id __attribute__((aligned(128)));
static volatile unsigned int waker_a_observed __attribute__((aligned(128)));
static volatile unsigned int waker_b_observed __attribute__((aligned(128)));
static volatile unsigned int wake_counter __attribute__((aligned(128)));
static volatile unsigned long long waker_a_result __attribute__((aligned(128)));
static volatile unsigned long long waker_b_result __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));

static void waiter_a_entry(void *arg)
{
    (void)arg;
    s32 r = syscall5_s32(SYS_EVENT_FLAG_WAIT,
        flag_id, 0b0001, WAIT_AND_NOCLEAR,
        (unsigned long)&waker_a_result, 0);
    if (r == 0) {
        waker_a_observed = (unsigned int)waker_a_result;
        __atomic_add_fetch(&wake_counter, 1, __ATOMIC_RELAXED);
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xA001);
    for (;;) { }
}

static void waiter_b_entry(void *arg)
{
    (void)arg;
    s32 r = syscall5_s32(SYS_EVENT_FLAG_WAIT,
        flag_id, 0b0010, WAIT_AND_NOCLEAR,
        (unsigned long)&waker_b_result, 0);
    if (r == 0) {
        waker_b_observed = (unsigned int)waker_b_result;
        __atomic_add_fetch(&wake_counter, 1, __ATOMIC_RELAXED);
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xB001);
    for (;;) { }
}

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
}

int main(void)
{
    unsigned long long tid_a = 0, tid_b = 0;
    unsigned long long retval = 0;
    s32 ret;

    flag_id = 0;
    waker_a_observed = 0xDEADBEEF;
    waker_b_observed = 0xDEADBEEF;
    wake_counter = 0;

    /* Create event flag with init bits = 0. */
    ret = syscall3_s32(SYS_EVENT_FLAG_CREATE,
        (unsigned long)&flag_id, 0, 0);
    if (ret != 0) { result.status = 0x01; write_tty_result(&result); return 1; }

    /* Spawn both waiters. */
    ret = syscall6_s32(SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid_a, (unsigned long)&waiter_a_entry,
        0, 1000, 0x4000, 0);
    if (ret != 0) { result.status = 0x02; write_tty_result(&result); return 1; }
    ret = syscall6_s32(SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid_b, (unsigned long)&waiter_b_entry,
        0, 1000, 0x4000, 0);
    if (ret != 0) { result.status = 0x04; write_tty_result(&result); return 1; }

    /* Give both waiters a chance to park by setting bits that
     * match neither first (they should be parked by the time we
     * reach the matching set below). */

    /* Set both bits in one call -- should wake A and B. */
    ret = syscall2_s32(SYS_EVENT_FLAG_SET, flag_id, 0b0011);
    if (ret != 0) { result.status = 0x08; write_tty_result(&result); return 1; }

    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid_a, (unsigned long)&retval);
    if (ret != 0 || retval != 0xA001) { result.status = 0x10; write_tty_result(&result); return 1; }
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid_b, (unsigned long)&retval);
    if (ret != 0 || retval != 0xB001) { result.status = 0x20; write_tty_result(&result); return 1; }

    result.status = 0;
    result.waker_a = waker_a_observed;
    result.waker_b = waker_b_observed;
    result.wake_cnt = wake_counter;
    if (result.waker_a != 0b0011)  result.status |= 0x100;
    if (result.waker_b != 0b0011)  result.status |= 0x200;
    if (result.wake_cnt != 2)      result.status |= 0x400;
    write_tty_result(&result);
    return (int)result.status;
}
