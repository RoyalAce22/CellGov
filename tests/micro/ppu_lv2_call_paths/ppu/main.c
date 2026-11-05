/* PPU probe: the round trip of LV2 calls on their error path, and of
 * calls the uncontended-cost probe did not time, each timed with mftb.
 *
 * Each loop makes 1 << 12 calls of one syscall, five runs a loop. The
 * probe creates no kernel object: every object call names an ID the
 * kernel never issued, so it returns ESRCH before it touches an object.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, the baseline
 *    1  sc 1   sys_process_getpid
 *    2  sc 351 sys_memory_get_page_attribute of the payload
 *    3  sc 94  sys_semaphore_post, bad ID
 *    4  sc 114 sys_semaphore_get_value, bad ID
 *    5  sc 102 sys_mutex_lock, bad ID, no timeout
 *    6  sc 104 sys_mutex_unlock, bad ID
 *    7  sc 139 sys_event_flag_get, bad ID
 *    8  sc 87  sys_event_flag_set, bad ID
 *    9  sc 86  sys_event_flag_trywait, bad ID
 *   10  sc 131 sys_event_queue_tryreceive, bad ID
 *   11  sc 108 sys_cond_signal, bad ID
 *   12  sc 138 sys_event_port_send, bad ID
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   32 + L     loop L's first nonzero return code, or 0
 *   48         loop L's last return code, for L = 1 (the process ID)
 *   59         runs a loop (5)
 *   60         0x5050_4C50 when the probe wrote every word
 *
 * Payload: 64 u64 words, 512 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PROCESS_GETPID               1
#define SYS_EVENT_FLAG_TRYWAIT           86
#define SYS_EVENT_FLAG_SET               87
#define SYS_SEMAPHORE_POST               94
#define SYS_MUTEX_LOCK                   102
#define SYS_MUTEX_UNLOCK                 104
#define SYS_COND_SIGNAL                  108
#define SYS_SEMAPHORE_GET_VALUE          114
#define SYS_EVENT_QUEUE_TRYRECEIVE       131
#define SYS_EVENT_PORT_SEND              138
#define SYS_EVENT_FLAG_GET               139
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147
#define SYS_MEMORY_GET_PAGE_ATTRIBUTE    351

/* An ID no kernel object of this process carries. */
#define BAD_ID      0x7FFFFFF0u
/* sys_event_flag_trywait's AND mode. */
#define WAIT_AND    0x1

#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define LOOPS   13
#define DONE    0x50504C50ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static volatile u64 page_attr[4] __attribute__((aligned(8)));
static volatile int sem_value __attribute__((aligned(8)));
static volatile u64 flag_value __attribute__((aligned(8)));
static volatile u64 flag_result __attribute__((aligned(8)));
static volatile u64 events[4] __attribute__((aligned(8)));
static volatile int event_count __attribute__((aligned(8)));

static u64 first_rc[LOOPS];
static u64 last_rc[LOOPS];

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static inline u64 timebase(void)
{
    u64 tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = PAYLOAD;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)out, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)out, len);
}

/* CALLS calls of syscall `num`; returns the time-base ticks they took.
 * Keeps the first nonzero and the last return code of the loop. */
static u64 __attribute__((noinline)) calls(unsigned loop, u64 num, u64 a, u64 b, u64 c, u64 d)
{
    u64 i, rc = 0, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        rc = (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
        bad = bad ? bad : (rc & 0xFFFFFFFFull);
    }
    t0 = timebase() - t0;
    if (bad && !first_rc[loop])
        first_rc[loop] = bad;
    last_rc[loop] = rc;
    return t0;
}

/* Run loop `loop` REPS times; keep its fastest and slowest run. */
static void timed(unsigned loop, u64 num, u64 a, u64 b, u64 c, u64 d)
{
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 t = calls(loop, num, a, b, c, d);
        if (t < lo)
            lo = t;
        if (t > hi)
            hi = t;
    }
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    timed(0, SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0);
    timed(1, SYS_PROCESS_GETPID, 0, 0, 0, 0);
    timed(2, SYS_MEMORY_GET_PAGE_ATTRIBUTE, EA(out), EA(page_attr), 0, 0);
    timed(3, SYS_SEMAPHORE_POST, BAD_ID, 1, 0, 0);
    timed(4, SYS_SEMAPHORE_GET_VALUE, BAD_ID, EA(&sem_value), 0, 0);
    timed(5, SYS_MUTEX_LOCK, BAD_ID, 0, 0, 0);
    timed(6, SYS_MUTEX_UNLOCK, BAD_ID, 0, 0, 0);
    timed(7, SYS_EVENT_FLAG_GET, BAD_ID, EA(&flag_value), 0, 0);
    timed(8, SYS_EVENT_FLAG_SET, BAD_ID, 1, 0, 0);
    timed(9, SYS_EVENT_FLAG_TRYWAIT, BAD_ID, 1, WAIT_AND, EA(&flag_result));
    event_count = -1;
    timed(10, SYS_EVENT_QUEUE_TRYRECEIVE, BAD_ID, EA(events), 1, EA(&event_count));
    timed(11, SYS_COND_SIGNAL, BAD_ID, 0, 0, 0);
    timed(12, SYS_EVENT_PORT_SEND, BAD_ID, 0, 0, 0);

    for (i = 0; i < LOOPS; i++)
        put(32 + i, first_rc[i]);
    put(48, last_rc[1]);
    put(59, REPS);
    put(60, DONE);
    write_frame();
    return 0;
}
