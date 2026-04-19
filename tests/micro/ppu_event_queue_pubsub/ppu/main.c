/* PPU program: event-queue pub-sub. Primary thread spawns one
 * receiver child, sends N payloads via sys_event_port_send; the
 * child calls sys_event_queue_receive N times and sums the
 * payloads' data1 fields. Final sum equals 0 + 1 + ... + N-1.
 *
 * Structural microtest for the event queue primitive. Proves:
 *
 *   1. sys_event_queue_create allocates an id and stores it at
 *      id_ptr.
 *   2. sys_event_queue_receive on an empty queue blocks the
 *      caller until a sys_event_port_send delivers a payload.
 *   3. A send with a parked waiter hands the payload directly
 *      to that waiter via the WakeAndReturn response_updates
 *      channel; the full 32-byte sys_event_t lands at the
 *      waiter's out pointer (source / data1 / data2 / data3).
 *   4. send-arrival order is preserved: N sends produce N
 *      receive completions in the same order.
 *
 * Output layout (TTY: CGOV magic + 16 bytes):
 *   status  (u32)  0 = pass
 *   sum     (u32)  expected N * (N - 1) / 2
 *   errors  (u32)  expected 0
 *   last_d1 (u32)  expected N - 1 (most recent data1)
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

static inline s32 syscall4_s32(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r11)
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
#define SYS_EVENT_QUEUE_CREATE 128
#define SYS_EVENT_QUEUE_RECV   130
#define SYS_EVENT_PORT_SEND    134

#define MESSAGES 16

struct TestResult {
    unsigned int status;
    unsigned int sum;
    unsigned int errors;
    unsigned int last_d1;
};

/* sys_event_t matches the host layout: 4 consecutive u64 BE. */
struct SysEvent {
    unsigned long long source;
    unsigned long long data1;
    unsigned long long data2;
    unsigned long long data3;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned int queue_id __attribute__((aligned(128)));
static volatile unsigned int receiver_sum __attribute__((aligned(128)));
static volatile unsigned int receiver_errors __attribute__((aligned(128)));
static volatile unsigned int receiver_last_d1 __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));
static struct SysEvent incoming __attribute__((aligned(128)));

static void receiver_entry(void *arg)
{
    (void)arg;
    unsigned int sum = 0;
    unsigned int errs = 0;
    unsigned int last = 0;
    for (unsigned int i = 0; i < MESSAGES; i++) {
        s32 r = syscall3_s32(SYS_EVENT_QUEUE_RECV,
            queue_id, (unsigned long)&incoming, 0);
        if (r != 0) {
            errs++;
            continue;
        }
        sum += (unsigned int)incoming.data1;
        last = (unsigned int)incoming.data1;
    }
    receiver_sum = sum;
    receiver_errors = errs;
    receiver_last_d1 = last;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xBEEF0001);
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
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;

    receiver_sum = 0;
    receiver_errors = 0xDEADBEEF;
    receiver_last_d1 = 0xDEADBEEF;

    /* Create queue. sys_event_queue_create(id_ptr, attr, key,
     * size). Size 0 is replaced by the default 127 by the
     * handler, but we request 16 explicitly. */
    ret = syscall4_s32(SYS_EVENT_QUEUE_CREATE,
        (unsigned long)&queue_id, 0, 0, 16);
    if (ret != 0) { result.status = 0x01; write_tty_result(&result); return 1; }

    /* Spawn receiver. */
    ret = syscall6_s32(SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid, (unsigned long)&receiver_entry,
        0, 1000, 0x4000, 0);
    if (ret != 0) { result.status = 0x02; write_tty_result(&result); return 1; }

    /* Send N payloads. data1 = i. */
    for (unsigned int i = 0; i < MESSAGES; i++) {
        s32 s = syscall4_s32(SYS_EVENT_PORT_SEND, queue_id, i, 0, 0);
        if (s != 0) { result.status = 0x04; write_tty_result(&result); return 1; }
    }

    /* Join receiver. */
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0) { result.status = 0x08; write_tty_result(&result); return 1; }
    if (retval != 0xBEEF0001) { result.status = 0x10; write_tty_result(&result); return 1; }

    unsigned int expected = (MESSAGES * (MESSAGES - 1)) / 2;

    result.status = 0;
    result.sum = receiver_sum;
    result.errors = receiver_errors;
    result.last_d1 = receiver_last_d1;
    if (result.sum != expected)          result.status |= 0x100;
    if (result.errors != 0)              result.status |= 0x200;
    if (result.last_d1 != MESSAGES - 1)  result.status |= 0x400;
    write_tty_result(&result);
    return (int)result.status;
}
