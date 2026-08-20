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

static inline s32 syscall8_s32(u64 num, u64 a, u64 b, u64 c, u64 d,
                               u64 e, u64 f, u64 g, u64 h)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r7 __asm__("7") = e;
    register u64 r8 __asm__("8") = f;
    register u64 r9 __asm__("9") = g;
    register u64 r10 __asm__("10") = h;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r7), "r"(r8), "r"(r9), "r"(r10), "r"(r11)
        : "r0", "r12", "cr0", "ctr", "memory"
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
#define SYS_EVENT_QUEUE_CREATE       128
#define SYS_EVENT_QUEUE_RECV         130
#define SYS_EVENT_PORT_CREATE        134
#define SYS_EVENT_PORT_CONNECT_LOCAL 136
#define SYS_EVENT_PORT_SEND          138

/* sys_event_port_create's port_type: LOCAL(1) connects by queue
 * id, IPC(3) by key (RPCS3 sys_event.cpp sys_event_port_create).
 * Sends go through a port connected to the queue, never through
 * the queue id directly. */
#define SYS_EVENT_PORT_LOCAL 1

/* sys_event_queue_attribute_t: protocol@0 u32, type@4 s32,
 * name@8. The kernel reads protocol (FIFO/PRIORITY) and type
 * (SYS_PPU_QUEUE 1 / SYS_SPU_QUEUE 2) out of the struct, so the
 * pointer must name valid memory (RPCS3 sys_event.cpp
 * sys_event_queue_create). */
#define SYS_PPU_QUEUE 1
static const struct {
    unsigned int protocol;      /* SYS_SYNC_FIFO */
    int type;                   /* SYS_PPU_QUEUE */
    char name[8];
} equeue_attr __attribute__((aligned(8))) = { 1, SYS_PPU_QUEUE, "" };

/* Syscall 52 takes 8 args: (thread_id*, param*, arg, unk, prio,
 * stacksize, flags, threadname*) per RPCS3 lv2.cpp /
 * sys_ppu_thread.cpp _sys_ppu_thread_create; liblv2's wrapper
 * passes unk = 0. The param* in r4 is a ppu_thread_param_t
 * { u32 entry_opd_ptr; u32 tls }, and the OPD it names is the
 * kernel's 8-byte { u32 code; u32 toc } form. The toolchain's
 * `&fn` resolves to the function's ELFv1 .opd descriptor -- 24
 * bytes of u64 fields -- so repack it before the syscall. */
struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr; /* -> opd_code below */
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

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

static struct cg_thread_param receiver_param __attribute__((aligned(8)));

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
static unsigned int port_id __attribute__((aligned(128)));
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
     * size); size must be 1..127 -- anything else is EINVAL
     * (RPCS3 sys_event.cpp sys_event_queue_create). */
    ret = syscall4_s32(SYS_EVENT_QUEUE_CREATE,
        (unsigned long)&queue_id, (unsigned long)&equeue_attr, 0, 16);
    if (ret != 0) { result.status = 0x01; write_tty_result(&result); return 1; }

    /* Create a LOCAL port and connect it to the queue. */
    ret = syscall3_s32(SYS_EVENT_PORT_CREATE,
        (unsigned long)&port_id, SYS_EVENT_PORT_LOCAL, 0);
    if (ret != 0) { result.status = 0x20; write_tty_result(&result); return 1; }
    ret = syscall2_s32(SYS_EVENT_PORT_CONNECT_LOCAL, port_id, queue_id);
    if (ret != 0) { result.status = 0x40; write_tty_result(&result); return 1; }

    /* Spawn receiver. */
    ret = syscall8_s32(SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid,
        make_thread_param(&receiver_param, (const void *)&receiver_entry),
        0, 0, 1000, 0x4000, 0, 0);
    if (ret != 0) { result.status = 0x02; write_tty_result(&result); return 1; }

    /* Send N payloads through the connected port. data1 = i. */
    for (unsigned int i = 0; i < MESSAGES; i++) {
        s32 s = syscall4_s32(SYS_EVENT_PORT_SEND, port_id, i, 0, 0);
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
