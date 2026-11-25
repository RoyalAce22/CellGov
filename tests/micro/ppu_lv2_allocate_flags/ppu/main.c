/* PPU probe: the round trip of sys_memory_allocate when its flags name no
 * page size, beside the same sizes with the 1 MiB page flag, and the
 * answer to a size whose upper word is not zero, each call timed alone
 * with mftb.
 *
 * Each of ROUNDS rounds makes the calls below in order, each between two
 * mftb reads. The time-base frequency call is the baseline, as in the
 * ppu_lv2_call_heavy probe. Each allocation that returns 0 is freed at
 * once.
 *
 *   allocation A (size, flags)
 *    0  1 MiB, 0x400 (1 MiB pages)
 *    1 16 MiB, 0x400 (1 MiB pages)
 *    2  1 MiB, 0     (no page size)
 *    3 16 MiB, 0     (no page size)
 *    4  0x1_0010_0000 (1 MiB, bit 32 set), 0x400
 *
 *   call
 *    0         sc 147 sys_time_get_timebase_frequency
 *    1 + 2A    sc 348 sys_memory_allocate(size, flags, &addr)
 *    2 + 2A    sc 349 sys_memory_free(addr), only after a 0 return
 *
 *   word (big-endian u64)
 *    C        call C's first nonzero return code, or 0; for call 0 the
 *             last frequency
 *    40 + A   allocation A's rounds that returned 0
 *    48       allocation 4's return code in round 0, as the kernel left r3
 *    56  ROUNDS (16)   57  CALLS (11)
 *    60  0x414C_4C46 when the probe wrote every word
 *   from byte 512: big-endian u32 samples, call C round R at
 *   512 + 4 * (C * ROUNDS + R), in time-base ticks; a free never sent
 *   records 0
 *
 * Payload: 1216 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147
#define SYS_MEMORY_ALLOCATE              348
#define SYS_MEMORY_FREE                  349

#define PAGE_1M  0x400

#define ALLOCS   5
#define ROUNDS   16
#define CALLS    (1 + 2 * ALLOCS)
#define PAYLOAD  (512 + 4 * CALLS * ROUNDS)
#define DONE     0x414C4C46ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int addr_word __attribute__((aligned(8)));

static u64 arg[8];
static u64 first_rc[CALLS];
static u64 succeeded[ALLOCS];
static u64 wide_rc;

static const u64 ALLOC_SIZE[ALLOCS] = {
    0x100000ull, 0x1000000ull, 0x100000ull, 0x1000000ull, 0x100100000ull,
};

static const u64 ALLOC_FLAGS[ALLOCS] = {
    PAGE_1M, PAGE_1M, 0, 0, PAGE_1M,
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
    if ((rc & 0xFFFFFFFFull) != 0 && !first_rc[call])
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

static void alloc_round(unsigned a, unsigned r)
{
    unsigned base = 1 + 2 * a;
    u64 rc;
    addr_word = 0;
    set_args(ALLOC_SIZE[a], ALLOC_FLAGS[a], EA(&addr_word));
    rc = one(base, r, SYS_MEMORY_ALLOCATE);
    if (a == 4 && r == 0)
        wide_rc = rc;
    if (rc & 0xFFFFFFFFull)
        return;
    succeeded[a]++;
    set_args(addr_word, 0, 0);
    one(base + 1, r, SYS_MEMORY_FREE);
}

int main(void)
{
    unsigned i, r, a;
    u64 frequency = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (r = 0; r < ROUNDS; r++) {
        set_args(0, 0, 0);
        frequency = one(0, r, SYS_TIME_GET_TIMEBASE_FREQUENCY);
        for (a = 0; a < ALLOCS; a++)
            alloc_round(a, r);
    }

    first_rc[0] = frequency;
    for (i = 0; i < CALLS; i++)
        put(i, first_rc[i]);
    for (a = 0; a < ALLOCS; a++)
        put(40 + a, succeeded[a]);
    put(48, wide_rc);
    put(56, ROUNDS);
    put(57, CALLS);
    put(60, DONE);
    write_frame();
    return 0;
}
