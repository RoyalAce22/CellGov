/* PPU probe: the round trip of LV2 calls that refuse an object ID the
 * kernel never issued, for calls whose success path the uncontended-cost
 * probe timed and whose error path the error-path probe did not, each
 * timed with mftb.
 *
 * Each loop makes 1 << 12 calls of one syscall, five runs a loop. The
 * probe creates no kernel object: every object call names an ID the
 * kernel never issued, so it returns ESRCH before it touches an object.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, the baseline
 *    1  sc 94  sys_semaphore_post, bad ID, as the error-path probe times it
 *    2  sc 93  sys_semaphore_trywait, bad ID
 *    3  sc 118 sys_event_flag_clear, bad ID, every bit
 *    4  sc 103 sys_mutex_trylock, bad ID
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   32 + L     loop L's first nonzero return code, or 0
 *   59         runs a loop (5)
 *   60         0x5050_4C52 when the probe wrote every word
 *
 * Payload: 64 u64 words, 512 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_SEMAPHORE_TRYWAIT            93
#define SYS_SEMAPHORE_POST               94
#define SYS_MUTEX_TRYLOCK                103
#define SYS_EVENT_FLAG_CLEAR             118
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

/* An ID no kernel object of this process carries. */
#define BAD_ID      0x7FFFFFF0u

#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define LOOPS   5
#define DONE    0x50504C52ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static u64 first_rc[LOOPS];

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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
 * Keeps the first nonzero return code of the loop. */
static u64 __attribute__((noinline)) calls(unsigned loop, u64 num, u64 a, u64 b, u64 c, u64 d)
{
    u64 i, rc, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        rc = (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
        bad = bad ? bad : (rc & 0xFFFFFFFFull);
    }
    t0 = timebase() - t0;
    if (bad && !first_rc[loop])
        first_rc[loop] = bad;
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
    timed(1, SYS_SEMAPHORE_POST, BAD_ID, 1, 0, 0);
    timed(2, SYS_SEMAPHORE_TRYWAIT, BAD_ID, 0, 0, 0);
    timed(3, SYS_EVENT_FLAG_CLEAR, BAD_ID, 0, 0, 0);
    timed(4, SYS_MUTEX_TRYLOCK, BAD_ID, 0, 0, 0);

    for (i = 0; i < LOOPS; i++)
        put(32 + i, first_rc[i]);
    put(59, REPS);
    put(60, DONE);
    write_frame();
    return 0;
}
