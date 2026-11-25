/* PPU probe: the round trip of the PRX module, thread and memory calls on
 * their success path, each call timed alone with mftb.
 *
 * Each of ITER rounds makes the calls below in order, each between two
 * mftb reads. The time-base frequency call is the baseline: its samples
 * give the cycles the bracket adds beside the cheapest call's round
 * trip.
 *
 * A module round loads /dev_flash/sys/external/libadec.sprx and runs the
 * start and stop handshakes with raw calls. It reports 0 (resident) to
 * each second phase and calls neither entry the first phases return.
 * A thread round creates a joinable thread at priority 2000, below the
 * main thread's 1001, and starts it. The thread counts itself and exits
 * at once on the other hardware thread. The main thread waits for the
 * count and 100 us more, then joins it.
 *
 *   call
 *    0  sc 147 sys_time_get_timebase_frequency
 *    1  sc 480 _sys_prx_load_module(path, 0, NULL)
 *    2  sc 481 _sys_prx_start_module(id, 0, opt), cmd 1
 *    3  sc 481 _sys_prx_start_module(id, 0, opt), cmd 2, res 0
 *    4  sc 482 _sys_prx_stop_module(id, 0, opt), cmd 1
 *    5  sc 482 _sys_prx_stop_module(id, 0, opt), cmd 2, res 0
 *    6  sc 483 _sys_prx_unload_module(id, 0, NULL)
 *    7  sc 52  _sys_ppu_thread_create, joinable, stack 0x4000
 *    8  sc 53  sys_ppu_thread_start
 *    9  sc 44  sys_ppu_thread_join
 *   10  sc 348 sys_memory_allocate(0x100000, 1M pages, &addr)
 *   11  sc 349 sys_memory_free(addr)
 *
 *   word (big-endian u64)
 *    C        call C's first nonzero return code, or 0; for call 0 the
 *             last frequency; for call 1 the first negative id, or 0
 *   16       module rounds whose load returned an id
 *   17       the entry word call 2 wrote, first round
 *   18       the entry word call 4 wrote, first round
 *   19       thread rounds whose thread counted itself
 *   20       the last joined thread's exit value (expected 0x5157)
 *   58  ITER (32)   59  CALLS (12)
 *   60  0x5050_4C48 when the probe wrote every word
 *   from byte 512: big-endian u32 samples, call C round R at
 *   512 + 4 * (C * ITER + R), in time-base ticks
 *
 * Payload: 2048 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define MAIN_PRIORITY  1001
#define LOWER_PRIORITY 2000

SYS_PROCESS_PARAM(MAIN_PRIORITY, 0x10000)

#define SYS_PPU_THREAD_EXIT              41
#define SYS_PPU_THREAD_JOIN              44
#define SYS_PPU_THREAD_CREATE            52
#define SYS_PPU_THREAD_START             53
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147
#define SYS_MEMORY_ALLOCATE              348
#define SYS_MEMORY_FREE                  349
#define SYS_PRX_LOAD_MODULE              480
#define SYS_PRX_START_MODULE             481
#define SYS_PRX_STOP_MODULE              482
#define SYS_PRX_UNLOAD_MODULE            483

#define JOINABLE 1
#define PAGE_1M  0x400
#define EXIT_VALUE 0x5157

#define ITER    32
#define CALLS   12
#define WORDS   64
#define PAYLOAD 2048
#define DONE    0x50504C48ull

/* 100 us of time base, past the thread's count, before the join. */
#define EXIT_WAIT_TB 7980ull
/* The main thread waits at most 1 s for the thread to count itself. */
#define START_WAIT_TB 79800000ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 opt[5] __attribute__((aligned(16)));
static volatile unsigned int addr_word __attribute__((aligned(8)));
static volatile unsigned int ran __attribute__((aligned(128)));

static u64 arg[8];
static u64 first_rc[CALLS];

static const char ADEC[] = "/dev_flash/sys/external/libadec.sprx";
static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static struct cg_thread_param child_param __attribute__((aligned(8)));

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

static void child_entry(void *a)
{
    (void)a;
    ran = ran + 1;
    __asm__ volatile ("sync" ::: "memory");
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, EXIT_VALUE);
    for (;;) { }
}

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

static void put_sample(unsigned call, unsigned round, u64 ticks)
{
    unsigned at = 512 + 4 * (call * ITER + round);
    unsigned i;
    if (ticks > 0xFFFFFFFFull)
        ticks = 0xFFFFFFFFull;
    for (i = 0; i < 4; i++)
        out[at + i] = (unsigned char)(ticks >> (24 - 8 * i));
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

/* One call of syscall `num` with arg[0..7], between two mftb reads.
 * Records the ticks as call `call`'s sample of round `round` and returns
 * the call's r3. */
static u64 __attribute__((noinline)) one(unsigned call, unsigned round, u64 num)
{
    u64 rc;
    u64 t0 = timebase();
    rc = (u64)cgov_sc(num, arg[0], arg[1], arg[2], arg[3], arg[4], arg[5], arg[6], arg[7]);
    t0 = timebase() - t0;
    put_sample(call, round, t0);
    if ((call == 1 ? (s32)rc < 0 : (rc & 0xFFFFFFFFull) != 0) && !first_rc[call])
        first_rc[call] = rc;
    return rc;
}

static void set_args(u64 a, u64 b, u64 c, u64 d, u64 e, u64 f, u64 g, u64 h)
{
    arg[0] = a; arg[1] = b; arg[2] = c; arg[3] = d;
    arg[4] = e; arg[5] = f; arg[6] = g; arg[7] = h;
}

static void set_opt(u64 cmd)
{
    opt[0] = 0x28;
    opt[1] = cmd;
    opt[2] = 0;
    opt[3] = 0;
    opt[4] = 0;
}

static u64 loaded, counted, frequency, exit_value;

static void module_round(unsigned r)
{
    s64 id;
    set_args(EA(ADEC), 0, 0, 0, 0, 0, 0, 0);
    id = (s64)(s32)one(1, r, SYS_PRX_LOAD_MODULE);
    if (id < 0)
        return;
    loaded++;
    set_opt(1);
    set_args((u64)id, 0, EA(opt), 0, 0, 0, 0, 0);
    one(2, r, SYS_PRX_START_MODULE);
    if (r == 0)
        put(17, opt[2]);
    set_opt(2);
    one(3, r, SYS_PRX_START_MODULE);
    set_opt(1);
    one(4, r, SYS_PRX_STOP_MODULE);
    if (r == 0)
        put(18, opt[2]);
    set_opt(2);
    one(5, r, SYS_PRX_STOP_MODULE);
    set_args((u64)id, 0, 0, 0, 0, 0, 0, 0);
    one(6, r, SYS_PRX_UNLOAD_MODULE);
}

static void thread_round(unsigned r)
{
    unsigned long long tid = 0, retval = 0;
    unsigned before = ran;
    u64 t0;
    set_args(EA(&tid), make_thread_param(&child_param, (const void *)&child_entry), 0, 0,
             LOWER_PRIORITY, 0x4000, JOINABLE, 0);
    if (one(7, r, SYS_PPU_THREAD_CREATE) & 0xFFFFFFFFull)
        return;
    set_args(tid, 0, 0, 0, 0, 0, 0, 0);
    if (one(8, r, SYS_PPU_THREAD_START) & 0xFFFFFFFFull)
        return;
    t0 = timebase();
    while (ran == before && timebase() - t0 < START_WAIT_TB) {
    }
    if (ran != before)
        counted++;
    t0 = timebase();
    while (timebase() - t0 < EXIT_WAIT_TB) {
    }
    set_args(tid, EA(&retval), 0, 0, 0, 0, 0, 0);
    one(9, r, SYS_PPU_THREAD_JOIN);
    exit_value = retval;
}

static void memory_round(unsigned r)
{
    addr_word = 0;
    set_args(0x100000, PAGE_1M, EA(&addr_word), 0, 0, 0, 0, 0);
    if (one(10, r, SYS_MEMORY_ALLOCATE) & 0xFFFFFFFFull)
        return;
    set_args(addr_word, 0, 0, 0, 0, 0, 0, 0);
    one(11, r, SYS_MEMORY_FREE);
}

int main(void)
{
    unsigned i, r;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (r = 0; r < ITER; r++) {
        set_args(0, 0, 0, 0, 0, 0, 0, 0);
        frequency = one(0, r, SYS_TIME_GET_TIMEBASE_FREQUENCY);
        module_round(r);
        thread_round(r);
        memory_round(r);
    }

    first_rc[0] = frequency;
    for (i = 0; i < CALLS; i++)
        put(i, first_rc[i]);
    put(16, loaded);
    put(19, counted);
    put(20, exit_value);
    put(58, ITER);
    put(59, CALLS);
    put(60, DONE);
    write_frame();
    return 0;
}
