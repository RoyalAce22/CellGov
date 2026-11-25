/* PPU probe: how the round trip of the PRX module calls grows with the
 * module, and of the memory allocate and free calls with the size and
 * the page size, each call timed alone with mftb.
 *
 * Each of ROUNDS rounds makes the calls below in order, each between two
 * mftb reads. The time-base frequency call is the baseline, as in the
 * ppu_lv2_call_heavy probe. Each module round loads one module from
 * /dev_flash/sys/external and runs the start and stop handshakes with
 * raw calls. It reports 0 (resident) to each second phase and calls
 * neither entry the first phases return. Each allocation round
 * allocates one block and frees it.
 *
 *   module M (file size in bytes)
 *    0  libfiber.sprx         16526
 *    1  libadec.sprx          35728
 *    2  libpngdec.sprx       171726
 *    3  libgem.sprx          281545
 *    4  libavcenc_small.sprx 973170
 *
 *   allocation A (size, page size)
 *    0  1 MiB, 1 MiB pages    3  64 KiB, 64 KiB pages
 *    1  4 MiB, 1 MiB pages    4   1 MiB, 64 KiB pages
 *    2 16 MiB, 1 MiB pages    5   4 MiB, 64 KiB pages
 *
 *   call
 *    0         sc 147 sys_time_get_timebase_frequency
 *    1 + 6M    sc 480 _sys_prx_load_module(path, 0, NULL)
 *    2 + 6M    sc 481 _sys_prx_start_module(id, 0, opt), cmd 1
 *    3 + 6M    sc 481, cmd 2, res 0
 *    4 + 6M    sc 482 _sys_prx_stop_module(id, 0, opt), cmd 1
 *    5 + 6M    sc 482, cmd 2, res 0
 *    6 + 6M    sc 483 _sys_prx_unload_module(id, 0, NULL)
 *   31 + 2A    sc 348 sys_memory_allocate(size, page size, &addr)
 *   32 + 2A    sc 349 sys_memory_free(addr)
 *
 *   word (big-endian u64)
 *    C        call C's first nonzero return code, or 0; for call 0 the
 *             last frequency; for a load the first negative id, or 0
 *   48 + M   module M's rounds whose load returned an id
 *   56  ROUNDS (16)   57  CALLS (43)
 *   60  0x5050_4C5A when the probe wrote every word
 *   from byte 512: big-endian u32 samples, call C round R at
 *   512 + 4 * (C * ROUNDS + R), in time-base ticks
 *
 * Payload: 3264 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147
#define SYS_MEMORY_ALLOCATE              348
#define SYS_MEMORY_FREE                  349
#define SYS_PRX_LOAD_MODULE              480
#define SYS_PRX_START_MODULE             481
#define SYS_PRX_STOP_MODULE              482
#define SYS_PRX_UNLOAD_MODULE            483

#define PAGE_1M  0x400
#define PAGE_64K 0x200

#define MODULES  5
#define ALLOCS   6
#define ROUNDS   16
#define CALLS    (1 + 6 * MODULES + 2 * ALLOCS)
#define ALLOC_CALL (1 + 6 * MODULES)
#define PAYLOAD  (512 + 4 * CALLS * ROUNDS)
#define DONE     0x50504C5Aull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 opt[5] __attribute__((aligned(16)));
static volatile unsigned int addr_word __attribute__((aligned(8)));

static u64 arg[8];
static u64 first_rc[CALLS];
static u64 loaded[MODULES];

static const char *const MODULE_PATH[MODULES] = {
    "/dev_flash/sys/external/libfiber.sprx",
    "/dev_flash/sys/external/libadec.sprx",
    "/dev_flash/sys/external/libpngdec.sprx",
    "/dev_flash/sys/external/libgem.sprx",
    "/dev_flash/sys/external/libavcenc_small.sprx",
};

static const u64 ALLOC_SIZE[ALLOCS] = {
    0x100000, 0x400000, 0x1000000, 0x10000, 0x100000, 0x400000,
};

static const u64 ALLOC_PAGE[ALLOCS] = {
    PAGE_1M, PAGE_1M, PAGE_1M, PAGE_64K, PAGE_64K, PAGE_64K,
};

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

static void put_sample(unsigned call, unsigned round, u64 ticks)
{
    unsigned at = 512 + 4 * (call * ROUNDS + round);
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

/* A load call's slot: 1, 7, 13, ... */
static int is_load(unsigned call)
{
    return call >= 1 && call < ALLOC_CALL && (call - 1) % 6 == 0;
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
    if ((is_load(call) ? (s32)rc < 0 : (rc & 0xFFFFFFFFull) != 0) && !first_rc[call])
        first_rc[call] = rc;
    return rc;
}

static void set_args(u64 a, u64 b, u64 c)
{
    unsigned i;
    arg[0] = a;
    arg[1] = b;
    arg[2] = c;
    for (i = 3; i < 8; i++)
        arg[i] = 0;
}

static void set_opt(u64 cmd)
{
    opt[0] = 0x28;
    opt[1] = cmd;
    opt[2] = 0;
    opt[3] = 0;
    opt[4] = 0;
}

static void module_round(unsigned m, unsigned r)
{
    unsigned base = 1 + 6 * m;
    s64 id;
    set_args(EA(MODULE_PATH[m]), 0, 0);
    id = (s64)(s32)one(base, r, SYS_PRX_LOAD_MODULE);
    if (id < 0)
        return;
    loaded[m]++;
    set_opt(1);
    set_args((u64)id, 0, EA(opt));
    one(base + 1, r, SYS_PRX_START_MODULE);
    set_opt(2);
    one(base + 2, r, SYS_PRX_START_MODULE);
    set_opt(1);
    one(base + 3, r, SYS_PRX_STOP_MODULE);
    set_opt(2);
    one(base + 4, r, SYS_PRX_STOP_MODULE);
    set_args((u64)id, 0, 0);
    one(base + 5, r, SYS_PRX_UNLOAD_MODULE);
}

static void alloc_round(unsigned a, unsigned r)
{
    unsigned base = ALLOC_CALL + 2 * a;
    addr_word = 0;
    set_args(ALLOC_SIZE[a], ALLOC_PAGE[a], EA(&addr_word));
    if (one(base, r, SYS_MEMORY_ALLOCATE) & 0xFFFFFFFFull)
        return;
    set_args(addr_word, 0, 0);
    one(base + 1, r, SYS_MEMORY_FREE);
}

int main(void)
{
    unsigned i, r, m, a;
    u64 frequency = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (r = 0; r < ROUNDS; r++) {
        set_args(0, 0, 0);
        frequency = one(0, r, SYS_TIME_GET_TIMEBASE_FREQUENCY);
        for (m = 0; m < MODULES; m++)
            module_round(m, r);
        for (a = 0; a < ALLOCS; a++)
            alloc_round(a, r);
    }

    first_rc[0] = frequency;
    for (i = 0; i < CALLS; i++)
        put(i, first_rc[i]);
    for (m = 0; m < MODULES; m++)
        put(48 + m, loaded[m]);
    put(56, ROUNDS);
    put(57, CALLS);
    put(60, DONE);
    write_frame();
    return 0;
}
