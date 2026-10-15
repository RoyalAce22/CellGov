/* PPU probe: the mmapper allocate answers a game process gets:
 * sys_mmapper_allocate_address (330), sys_mmapper_allocate_shared_memory
 * (332), sys_mmapper_allocate_shared_memory_ext (339) and
 * sys_mmapper_allocate_shared_memory_from_container (362).
 *
 * Each address area or shared segment the probe gets is freed at once
 * (331 for an area, 329 for a segment), so at most one is live. Areas
 * are 256 MiB of address space; segments are 64 KiB or 1 MiB.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 * Each "rc, word, free rc" triple holds the call's rc, the out word
 * (pre-filled 0xFFFFFFFF) and the rc of the free, or SKIP when the call
 * failed and nothing was freed.
 *   A. sc 330(size, flags, alignment, &addr)
 *    0-2   size 0x1000, 1M flags, alignment 256M
 *    3-5   size 256M, 1M flags, alignment 256M, NULL out pointer
 *          (slot 4 holds 0, slot 5 the free of nothing: SKIP)
 *    6-8   size 256M, 1M flags, alignment 0
 *    9-11  size 256M, 1M flags, alignment 0x3000
 *   12-14  size 256M, 1M flags, alignment 256M
 *   B. sc 332(key, size, flags, &id)
 *   15-17  keyless, size 0, 1M flags
 *   18-20  keyless, 1M, 1M flags, NULL out pointer (as slots 3-5)
 *   21-23  keyless, 1M, flags 0x300
 *   24     key 0, 1M, 1M flags: rc      25  the id
 *   26     key 0 again: rc              27  the id
 *   28     sc 329 of the first id       29  sc 329 of the second id
 *   30-32  keyless, 64K, 64K flags
 *   C. sc 339(key, 64K, 64K flags, entries, count, &id)
 *   33-35  key K, type 0, count 1 (slot 35 frees it after slot 36)
 *   36     key K, type 0, count 1, while K is held: rc
 *          (slot 67 frees that segment if it was granted, else SKIP)
 *   37-39  keyless, type 1, count 1
 *   40-42  keyless, type 2, count 1
 *   43-45  keyless, type 3, count 1
 *   46-48  keyless, type 4, count 1
 *   49-51  keyless, type 5, count 1
 *   52-54  keyless, type 6, count 1
 *   55-57  keyless, type 0, count 0
 *   58-60  keyless, type 0, count 16
 *   61-63  keyless, type 0, count 17
 *   D. sc 362(keyless, 64K, cid, 64K flags, &id)
 *   64-66  cid 0xDEADBEEF
 *   67     the free of slot 36's segment (see above)
 *
 * SKIP is 0xDEADD00D. K is 0x0000C0DE14860001, a key no system
 * service uses.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MMAPPER_ALLOCATE_ADDRESS     330
#define SYS_MMAPPER_FREE_ADDRESS         331
#define SYS_MMAPPER_ALLOCATE_SHM         332
#define SYS_MMAPPER_FREE_SHM             329
#define SYS_MMAPPER_ALLOCATE_SHM_EXT     339
#define SYS_MMAPPER_ALLOCATE_SHM_FROM_CT 362

#define KEYLESS   0xFFFF000000000000ULL
#define KEY_K     0x0000C0DE14860001ULL
#define FLAG_64K  0x200
#define FLAG_1M   0x400
#define AREA      0x10000000ULL
#define MIB       0x100000ULL
#define K64       0x10000ULL
#define SKIP      0xDEADD00DULL

#define WORDS   68
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile unsigned int w32b __attribute__((aligned(8)));
/* sys_mmapper ext entries: 0x18 bytes each, the type u64 at +0x10. */
static volatile unsigned long long entries[17 * 3] __attribute__((aligned(16)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
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

/* rc, word, free rc at `slot`. */
static void triple(unsigned slot, u64 rc, u64 word, unsigned free_num)
{
    put(slot, rc);
    put(slot + 1, word);
    put(slot + 2, rc == 0 ? sc(free_num, word, 0, 0, 0) : SKIP);
}

static void area(unsigned slot, u64 size, u64 alignment, int null_out)
{
    u64 rc;
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, size, FLAG_1M, alignment, null_out ? 0 : EA(&w32));
    if (null_out) {
        put(slot, rc);
        put(slot + 1, 0);
        put(slot + 2, SKIP);
        return;
    }
    triple(slot, rc, w32, SYS_MMAPPER_FREE_ADDRESS);
}

static void shm(unsigned slot, u64 key, u64 size, u64 flags, int null_out)
{
    u64 rc;
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_SHM, key, size, flags, null_out ? 0 : EA(&w32));
    if (null_out) {
        put(slot, rc);
        put(slot + 1, 0);
        put(slot + 2, SKIP);
        return;
    }
    triple(slot, rc, w32, SYS_MMAPPER_FREE_SHM);
}

static u64 ext(u64 key, u64 type, u64 count, volatile unsigned int *id)
{
    unsigned i;
    for (i = 0; i < 17 * 3; i++)
        entries[i] = 0;
    for (i = 0; i < 17; i++)
        entries[i * 3 + 2] = type;
    *id = 0xFFFFFFFFu;
    return (u64)cgov_sc(SYS_MMAPPER_ALLOCATE_SHM_EXT, key, K64, FLAG_64K, EA(entries), count,
                        EA(id), 0, 0);
}

static void ext_triple(unsigned slot, u64 type, u64 count)
{
    u64 rc = ext(KEYLESS, type, count, &w32);
    triple(slot, rc, w32, SYS_MMAPPER_FREE_SHM);
}

int main(void)
{
    unsigned i;
    u64 rc, id1, id2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    /* A. */
    area(0, 0x1000, AREA, 0);
    area(3, AREA, AREA, 1);
    area(6, AREA, 0, 0);
    area(9, AREA, 0x3000, 0);
    area(12, AREA, AREA, 0);

    /* B. */
    shm(15, KEYLESS, 0, FLAG_1M, 0);
    shm(18, KEYLESS, MIB, FLAG_1M, 1);
    shm(21, KEYLESS, MIB, 0x300, 0);
    w32 = 0xFFFFFFFFu;
    put(24, sc(SYS_MMAPPER_ALLOCATE_SHM, 0, MIB, FLAG_1M, EA(&w32)));
    id1 = w32;
    put(25, id1);
    w32 = 0xFFFFFFFFu;
    put(26, sc(SYS_MMAPPER_ALLOCATE_SHM, 0, MIB, FLAG_1M, EA(&w32)));
    id2 = w32;
    put(27, id2);
    put(28, sc(SYS_MMAPPER_FREE_SHM, id1, 0, 0, 0));
    put(29, sc(SYS_MMAPPER_FREE_SHM, id2, 0, 0, 0));
    shm(30, KEYLESS, K64, FLAG_64K, 0);

    /* C. The key K stays held across slot 36, then slot 35 frees it. */
    rc = ext(KEY_K, 0, 1, &w32);
    id1 = w32;
    put(33, rc);
    put(34, id1);
    id2 = ext(KEY_K, 0, 1, &w32b);
    put(36, id2);
    put(67, id2 == 0 ? sc(SYS_MMAPPER_FREE_SHM, w32b, 0, 0, 0) : SKIP);
    put(35, rc == 0 ? sc(SYS_MMAPPER_FREE_SHM, id1, 0, 0, 0) : SKIP);
    ext_triple(37, 1, 1);
    ext_triple(40, 2, 1);
    ext_triple(43, 3, 1);
    ext_triple(46, 4, 1);
    ext_triple(49, 5, 1);
    ext_triple(52, 6, 1);
    ext_triple(55, 0, 0);
    ext_triple(58, 0, 16);
    ext_triple(61, 0, 17);

    /* D. */
    w32 = 0xFFFFFFFFu;
    rc = (u64)cgov_sc(SYS_MMAPPER_ALLOCATE_SHM_FROM_CT, KEYLESS, K64, 0xDEADBEEFu, FLAG_64K,
                      EA(&w32), 0, 0, 0);
    triple(64, rc, w32, SYS_MMAPPER_FREE_SHM);

    write_frame();
    return 0;
}
