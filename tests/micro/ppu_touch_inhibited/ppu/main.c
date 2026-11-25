/* PPU probe: whether a touch of a caching-inhibited line places a line in
 * the L1 data cache. RSX local memory is caching-inhibited for a game
 * process, so libgcm brings up an RSX context and the probe touches a
 * line of local memory. It writes nothing to local memory.
 *
 * Four cacheable lines A0..A3 lie 8 KB apart, so they fill the four ways
 * of one L1 set. Each Ai holds the address of Ai+1, and A3 holds END.
 * Each iteration of a run:
 *   1. loads A0..A3 twice, untimed, so the four lines fill the set;
 *   2. runs one of five cases on a fifth line of the same set:
 *        case
 *        0  no touch
 *        1  dcbt   0,B  (B is cacheable, 8 KB after A3)
 *        2  dcbt   0,T  (T is in RSX local memory)
 *        3  dcbtst 0,T
 *        4  dcbt   0,U  (U is in page 0, which no page holds)
 *   3. waits WAIT time base ticks, so a line that the touch requests
 *      arrives;
 *   4. times the chase A0 -> A1 -> A2 -> A3 -> END with mftb.
 * A touch that fills a line of the set drops one of A0..A3, and the
 * chase reloads it from the L2. A run is ITER iterations, and its time is
 * the sum of their time base deltas. Each case runs REPS times,
 * interleaved with the other cases.
 *
 * localAddress is a 32-bit pointer; the probe zero-extends it. Before the
 * runs, an lwz of T shows that T is mapped: a fault there stops the run
 * at the progress record of step 1.
 *
 * Each case writes a progress record naming it before its first touch,
 * so a run that stops names the case that stopped it; the final frame
 * replaces the record.
 *
 * Payload (20 big-endian u64):
 *   0       DONE once every case ran, PROG in a progress record
 *   1       the step a progress record names (0 in the final frame):
 *           1 the lwz of T, 2 + c case c
 *   2       rsxInit rc (0xFFFFFFFFFFFFFFFF when not sent)
 *   3       local memory address from gcmGetConfiguration, zero-extended
 *   4..8    the smallest run of cases 0 to 4, in time base ticks
 *   9..13   the largest run of cases 0 to 4, in time base ticks
 *   14      REPS
 *   15      ITER
 *   16      T
 *   17      U
 *   18      every chase ended at END (1 = yes)
 *   19      the word the lwz of T read
 * SKIP is 0xDEADD00D.
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000

#define DONE 0x444F4E45ULL
#define PROG 0x50524F47ULL
#define SKIP 0xDEADD00DULL
#define END  0x454E44454E44454EULL

#define WORDS   20
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CASES   5
#define ITER    4096
#define WAIT    100

/* One L1 way: 64 sets of 128-byte lines. */
#define WAY     8192ul
/* The set the probe uses, as a byte offset in a way. */
#define SET_OFF (5ul * 128ul)
/* T's offset in local memory: a whole number of ways, plus SET_OFF. */
#define T_OFF   (0x80000ul + SET_OFF)
/* U: page 0, in the set. */
#define U_EA    SET_OFF

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile u64 ways[5 * WAY / 8] __attribute__((aligned(8192)));
static gcmContextData *context;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static void progress(u64 step)
{
    result[1] = step;
    CGOV_OUT_FILE_WRITE((const void *)result, PAYLOAD);
}

static inline unsigned long long tb(void)
{
    unsigned long long t;
    __asm__ volatile ("mftb %0" : "=r"(t) : : "memory");
    return t;
}

static inline void wait_ticks(unsigned long long n)
{
    unsigned long long t0 = tb();
    while (tb() - t0 < n)
        ;
}

/* The address of line k of the set: A0..A3 for k = 0..3, B for k = 4. */
static unsigned long line(unsigned int k)
{
    return (unsigned long)&ways[0] + k * WAY + SET_OFF;
}

/* The chase A0 -> A1 -> A2 -> A3 -> END, untimed. */
static inline u64 chase(unsigned long a0)
{
    u64 p;
    __asm__ volatile ("ld %0,0(%1)\n\t"
                      "ld %0,0(%0)\n\t"
                      "ld %0,0(%0)\n\t"
                      "ld %0,0(%0)"
                      : "=&b"(p) : "b"(a0) : "memory");
    return p;
}

/* One run of case `c`: returns the summed time base ticks of its ITER
 * timed chases and clears `*ok` when a chase does not end at END. */
static unsigned int run(unsigned long t, unsigned int c, unsigned int *ok)
{
    unsigned long a0 = line(0);
    unsigned long b = line(4);
    unsigned int i, sum = 0;
    for (i = 0; i < ITER; i++) {
        unsigned long long t0, t1, use;
        u64 p;
        if (chase(a0) != END || chase(a0) != END)
            *ok = 0;
        if (c == 1)
            __asm__ volatile ("dcbt 0,%0" : : "r"(b) : "memory");
        else if (c == 2)
            __asm__ volatile ("dcbt 0,%0" : : "r"(t) : "memory");
        else if (c == 3)
            __asm__ volatile ("dcbtst 0,%0" : : "r"(t) : "memory");
        else if (c == 4)
            __asm__ volatile ("dcbt 0,%0" : : "r"(U_EA) : "memory");
        wait_ticks(WAIT);
        __asm__ volatile ("mftb %0\n\t"
                          "ld %1,0(%4)\n\t"
                          "ld %1,0(%1)\n\t"
                          "ld %1,0(%1)\n\t"
                          "ld %1,0(%1)\n\t"
                          "add %2,%1,%1\n\t"
                          "mftb %3"
                          : "=&r"(t0), "=&b"(p), "=&r"(use), "=&r"(t1)
                          : "b"(a0) : "memory");
        if (p != END)
            *ok = 0;
        sum += (unsigned int)(t1 - t0);
    }
    return sum;
}

int main(void)
{
    sys_mem_addr_t host = 0;
    gcmConfiguration config;
    unsigned long local, t;
    unsigned int r, c, k, ok = 1;
    u64 v;

    result[0] = PROG;
    result[1] = 0;
    result[2] = ~0ULL;
    for (k = 3; k < WORDS; k++)
        result[k] = SKIP;
    result[14] = REPS;
    result[15] = ITER;
    result[17] = U_EA;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        write_frame();
        return 1;
    }
    result[2] = (u64)(s64)rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    if (result[2] != 0) {
        write_frame();
        return 1;
    }
    gcmGetConfiguration(&config);
    local = (unsigned long)(volatile u64 *)config.localAddress;
    t = local + T_OFF;
    result[3] = local;
    result[16] = t;

    progress(1);
    __asm__ volatile ("lwz %0,0(%1)" : "=r"(v) : "b"(t) : "memory");
    result[19] = v;

    for (k = 0; k < 3; k++)
        *(volatile u64 *)line(k) = line(k + 1);
    *(volatile u64 *)line(3) = END;
    *(volatile u64 *)line(4) = END;
    __sync_synchronize();

    for (c = 0; c < CASES; c++) {
        result[4 + c] = ~0ULL;
        result[9 + c] = 0;
    }
    for (r = 0; r < REPS; r++) {
        for (c = 0; c < CASES; c++) {
            unsigned int ticks;
            if (r == 0)
                progress(2 + c);
            ticks = run(t, c, &ok);
            if (ticks < result[4 + c])
                result[4 + c] = ticks;
            if (ticks > result[9 + c])
                result[9 + c] = ticks;
        }
    }
    result[18] = ok;

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
