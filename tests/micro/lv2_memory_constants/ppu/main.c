/* PPU probe: the sizes, alignments, granules and flag words the memory
 * and mmapper calls accept from a game process.
 *
 * Every area, container, segment and block the probe gets is freed or
 * destroyed at once, so at most one of each is live (the 64 KiB segment
 * group C maps stays live through C only).
 *
 * Payload: big-endian u64 words, one per slot. A slot never sent holds
 * SKIP (0xDEADD00D).
 *   A. sc 330(size, 1M flags, alignment, &addr), then 331 of what it got:
 *      per case rc, address, free rc; cases in order
 *      (size, alignment) = (0x0FFFFFFF, 256M) (0x18000000, 256M)
 *      (0x20000000, 256M) (256M, 0x08000000) (256M, 0x20000000)
 *      (256M, 0x40000000) (256M, 0x80000000) (256M, 0x30000000)
 *        0-23
 *   B. sc 341(&cid, size), then 342: per case rc, cid, available memory
 *      after the create, destroy rc; sizes 0xFFFFF, 0x100000, 0x1FFFFF
 *       24-35
 *       36  available memory before group B
 *   C. sc 339(keyless, 64K, 64K flags, table, count, &id), then 329:
 *      per case rc, id, free rc
 *       37-39  one entry of type 7
 *       40-42  one entry of type 8
 *       43-45  count -1
 *       46-48  count 2, type 2 at table + 0x28, every other word 0
 *       49-51  count 2, type 2 at table + 0x30, every other word 0
 *       52-54  one entry of type 0 ending at the end of a mapped 64 KiB
 *              page (table = page end - 0x18)
 *       55-57  one entry of type 0 at page end - 0x17
 *       58  330 area rc     59  332 64K segment rc   60  334 map rc
 *       61  335 unmap rc    62  329 free rc          63  331 free rc
 *   D. sc 348(1M, flags, &addr), sc 351 on the block, then 349: per flags
 *      word rc, address, 351 rc, the three u64 words 351 wrote
 *      (pre-filled with 0xEE), free rc; flags 0, 0x100, 0x200, 0x400,
 *      0x600, 0x800, 0xF00
 *       64-112
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MMAPPER_FREE_SHM         329
#define SYS_MMAPPER_ALLOCATE_ADDRESS 330
#define SYS_MMAPPER_FREE_ADDRESS     331
#define SYS_MMAPPER_ALLOCATE_SHM     332
#define SYS_MMAPPER_MAP_SHM          334
#define SYS_MMAPPER_UNMAP_SHM        335
#define SYS_MMAPPER_ALLOCATE_SHM_EXT 339
#define SYS_MEMORY_CONTAINER_CREATE  341
#define SYS_MEMORY_CONTAINER_DESTROY 342
#define SYS_MEMORY_ALLOCATE          348
#define SYS_MEMORY_FREE              349
#define SYS_MEMORY_GET_PAGE_ATTR     351
#define SYS_MEMORY_GET_USER_MEMORY   352

#define KEYLESS   0xFFFF000000000000ULL
#define FLAG_64K  0x200
#define FLAG_1M   0x400
#define PROT_RW   0x40000
#define AREA      0x10000000ULL
#define MIB       0x100000ULL
#define K64       0x10000ULL
#define SKIP      0xDEADD00DULL

#define WORDS   113
#define PAYLOAD (WORDS * 8)

static const u64 AREA_CASES[8][2] = {
    { 0x0FFFFFFFULL, AREA }, { 0x18000000ULL, AREA }, { 0x20000000ULL, AREA },
    { AREA, 0x08000000ULL }, { AREA, 0x20000000ULL }, { AREA, 0x40000000ULL },
    { AREA, 0x80000000ULL }, { AREA, 0x30000000ULL },
};
static const u64 CONTAINER_SIZES[3] = { 0xFFFFFULL, 0x100000ULL, 0x1FFFFFULL };
static const u64 ALLOC_FLAGS[7] = { 0, 0x100, 0x200, 0x400, 0x600, 0x800, 0xF00 };

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile unsigned int info[2] __attribute__((aligned(8)));
static volatile unsigned long long table[8] __attribute__((aligned(16)));
static volatile unsigned char attr[24] __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static u64 sc6(u64 num, u64 a, u64 b, u64 c, u64 d, u64 e, u64 f)
{
    return (u64)cgov_sc(num, a, b, c, d, e, f, 0, 0);
}

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

static u64 raw8(const volatile unsigned char *p)
{
    u64 v = 0;
    unsigned i;
    for (i = 0; i < 8; i++)
        v = v << 8 | p[i];
    return v;
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

static u64 avail(void)
{
    info[0] = info[1] = 0xFFFFFFFFu;
    (void)sc(SYS_MEMORY_GET_USER_MEMORY, EA(info), 0, 0, 0);
    return info[1];
}

/* One 339 at `slot` from the table at `at`: rc, id, free rc. */
static void ext(unsigned slot, u64 at, u64 count)
{
    u64 rc;
    w32 = 0xFFFFFFFFu;
    rc = sc6(SYS_MMAPPER_ALLOCATE_SHM_EXT, KEYLESS, K64, FLAG_64K, at, count, EA(&w32));
    put(slot, rc);
    put(slot + 1, w32);
    put(slot + 2, rc == 0 ? sc(SYS_MMAPPER_FREE_SHM, w32, 0, 0, 0) : SKIP);
}

static void clear_table(void)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        table[i] = 0;
}

/* A one-entry table of `type` at the guest address `at`. */
static void entry_at(u64 at, u64 type)
{
    volatile unsigned char *p = (volatile unsigned char *)(unsigned long)at;
    unsigned i;
    for (i = 0; i < 0x18; i++)
        p[i] = 0;
    for (i = 0; i < 8; i++)
        p[0x10 + i] = (unsigned char)(type >> (56 - 8 * i));
}

int main(void)
{
    unsigned i, j;
    u64 rc, a, m, page_end;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < WORDS; i++)
        put(i, SKIP);

    /* A. */
    for (i = 0; i < 8; i++) {
        w32 = 0xFFFFFFFFu;
        rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA_CASES[i][0], FLAG_1M, AREA_CASES[i][1], EA(&w32));
        put(3 * i, rc);
        put(3 * i + 1, w32);
        put(3 * i + 2, rc == 0 ? sc(SYS_MMAPPER_FREE_ADDRESS, w32, 0, 0, 0) : SKIP);
    }

    /* B. */
    put(36, avail());
    for (i = 0; i < 3; i++) {
        w32 = 0xFFFFFFFFu;
        rc = sc(SYS_MEMORY_CONTAINER_CREATE, EA(&w32), CONTAINER_SIZES[i], 0, 0);
        put(24 + 4 * i, rc);
        put(24 + 4 * i + 1, w32);
        put(24 + 4 * i + 2, avail());
        put(24 + 4 * i + 3, rc == 0 ? sc(SYS_MEMORY_CONTAINER_DESTROY, w32, 0, 0, 0) : SKIP);
    }

    /* C. */
    clear_table();
    table[2] = 7;
    ext(37, EA(table), 1);
    table[2] = 8;
    ext(40, EA(table), 1);
    table[2] = 0;
    ext(43, EA(table), (u64)-1LL);
    clear_table();
    table[5] = 2;
    ext(46, EA(table), 2);
    clear_table();
    table[6] = 2;
    ext(49, EA(table), 2);
    clear_table();

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_64K, AREA, EA(&w32));
    a = w32;
    put(58, rc);
    if (rc == 0) {
        w32 = 0xFFFFFFFFu;
        rc = sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, K64, FLAG_64K, EA(&w32));
        m = w32;
        put(59, rc);
        if (rc == 0) {
            rc = sc(SYS_MMAPPER_MAP_SHM, a, m, PROT_RW, 0);
            put(60, rc);
            if (rc == 0) {
                page_end = a + K64;
                entry_at(page_end - 0x18, 0);
                ext(52, page_end - 0x18, 1);
                entry_at(page_end - 0x18, 0);
                ext(55, page_end - 0x17, 1);
                w32 = 0xFFFFFFFFu;
                put(61, sc(SYS_MMAPPER_UNMAP_SHM, a, EA(&w32), 0, 0));
            }
            put(62, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
        }
        put(63, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
    }

    /* D. */
    for (i = 0; i < 7; i++) {
        unsigned s = 64 + 7 * i;
        w32 = 0xFFFFFFFFu;
        rc = sc(SYS_MEMORY_ALLOCATE, MIB, ALLOC_FLAGS[i], EA(&w32), 0);
        put(s, rc);
        put(s + 1, w32);
        if (rc != 0)
            continue;
        for (j = 0; j < 24; j++)
            attr[j] = 0xEE;
        put(s + 2, sc(SYS_MEMORY_GET_PAGE_ATTR, w32, EA(attr), 0, 0));
        put(s + 3, raw8(attr));
        put(s + 4, raw8(attr + 8));
        put(s + 5, raw8(attr + 16));
        put(s + 6, sc(SYS_MEMORY_FREE, w32, 0, 0, 0));
    }

    write_frame();
    return 0;
}
