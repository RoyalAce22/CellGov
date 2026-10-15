/* PPU probe: the mmapper map, search and unmap answers a game process
 * gets: sys_mmapper_map_shared_memory (334),
 * sys_mmapper_unmap_shared_memory (335) and sys_mmapper_search_and_map
 * (337), with the free answers (331, 329) while a segment is mapped.
 *
 * The probe allocates one 256 MiB address area A of 64 KiB pages (330:
 * the lv2_mmapper_map probe is the same with 1 MiB pages), one 64 KiB
 * segment M and one 1 MiB segment M1 (332), and frees all three at the
 * end. Every map that succeeds is unmapped before the next step.
 *
 * Payload: big-endian u64 words, one per slot. An "unmap" slot holds
 * the rc of the unmap of what the step before mapped, or SKIP when that
 * step mapped nothing. When the setup in slots 0-5 fails, every later
 * slot holds SKIP.
 *    0  330 area rc                     1  A
 *    2  332 64K segment rc              3  M
 *    4  332 1M segment rc               5  M1
 *    6  334 M at A, RW
 *    7  334 M at A again, RW (the window is mapped)
 *    8  334 M1 at A+64K, RW (not 1 MiB aligned)
 *    9  329 free M while mapped        10  331 free A while mapped
 *   11  335 unmap A                    12  the id it returns
 *   13  335 unmap A again
 *   14  334 M at A+0x100
 *   15  334 id 0xDEADBEEF at A
 *   16  334 M at A, flags 0x999        17  unmap
 *   18  334 M at A, RO                 19  unmap
 *   20  334 M at A +/- 256M (outside A, inside the window)  21  unmap
 *   22-23  SKIP (unused)
 *   24  337 from A, M, RW: rc          25  the address   26  unmap
 *   27  337 from A+64K: rc             28  the address   29  unmap
 *   30  337 from A, NULL out pointer   31  unmap A
 *   32  337 from A, id 0xDEADBEEF
 *   33  337 from A, M1: rc             34  the address
 *   35  337 from A, M1 again: rc       36  the address
 *   37  unmap of 34                    38  unmap of 35
 *   39  SKIP (unused)
 *   40  334 M at A                     41  335 unmap A, NULL id pointer
 *   42  335 unmap A
 *   43  329 free M    44  329 free M1    45  331 free A
 *   46  the address used by slot 20
 *   47  zero
 *
 * SKIP is 0xDEADD00D.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MMAPPER_ALLOCATE_ADDRESS 330
#define SYS_MMAPPER_FREE_ADDRESS     331
#define SYS_MMAPPER_ALLOCATE_SHM     332
#define SYS_MMAPPER_FREE_SHM         329
#define SYS_MMAPPER_MAP_SHM          334
#define SYS_MMAPPER_UNMAP_SHM        335
#define SYS_MMAPPER_SEARCH_AND_MAP   337

#define KEYLESS   0xFFFF000000000000ULL
#define FLAG_64K  0x200
#define FLAG_1M   0x400
#define PROT_RW   0x40000
#define PROT_RO   0x80000
#define AREA      0x10000000ULL
#define MIB       0x100000ULL
#define K64       0x10000ULL
#define BAD_ID    0xDEADBEEFULL
#define SKIP      0xDEADD00DULL

#define WORDS   48
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));

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

static u64 unmap(u64 addr)
{
    w32 = 0xFFFFFFFFu;
    return sc(SYS_MMAPPER_UNMAP_SHM, addr, EA(&w32), 0, 0);
}

/* A map at `addr` in `slot`, its unmap (or SKIP) in `slot + 1`. */
static void map_then_unmap(unsigned slot, u64 addr, u64 id, u64 flags)
{
    u64 rc = sc(SYS_MMAPPER_MAP_SHM, addr, id, flags, 0);
    put(slot, rc);
    put(slot + 1, rc == 0 ? unmap(addr) : SKIP);
}

/* A search from `start` in `slot`, the address in `slot + 1`, its
 * unmap (or SKIP) in `slot + 2`. */
static void search_then_unmap(unsigned slot, u64 start, u64 id)
{
    u64 rc, addr;
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_SEARCH_AND_MAP, start, id, PROT_RW, EA(&w32));
    addr = w32;
    put(slot, rc);
    put(slot + 1, addr);
    put(slot + 2, rc == 0 ? unmap(addr) : SKIP);
}

int main(void)
{
    unsigned i;
    u64 rc, rc2, a, m, m1, other, x1, x2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < WORDS - 1; i++)
        put(i, SKIP);

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_64K, AREA, EA(&w32));
    a = w32;
    put(0, rc);
    put(1, a);
    if (rc != 0)
        goto done;
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, K64, FLAG_64K, EA(&w32));
    m = w32;
    put(2, rc);
    put(3, m);
    if (rc != 0) {
        put(45, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        goto done;
    }
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, MIB, FLAG_1M, EA(&w32));
    m1 = w32;
    put(4, rc);
    put(5, m1);
    if (rc != 0) {
        put(43, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
        put(45, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        goto done;
    }

    /* 334 and the frees while mapped. */
    rc = sc(SYS_MMAPPER_MAP_SHM, a, m, PROT_RW, 0);
    put(6, rc);
    rc2 = sc(SYS_MMAPPER_MAP_SHM, a, m, PROT_RW, 0);
    put(7, rc2);
    x1 = sc(SYS_MMAPPER_MAP_SHM, a + K64, m1, PROT_RW, 0);
    put(8, x1);
    if (x1 == 0)
        unmap(a + K64);
    if (rc == 0) {
        put(9, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
        put(10, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        put(11, unmap(a));
        put(12, w32);
        put(13, unmap(a));
    }
    x1 = sc(SYS_MMAPPER_MAP_SHM, a + 0x100, m, PROT_RW, 0);
    put(14, x1);
    if (x1 == 0)
        unmap(a + 0x100);
    x1 = sc(SYS_MMAPPER_MAP_SHM, a, BAD_ID, PROT_RW, 0);
    put(15, x1);
    if (x1 == 0)
        unmap(a);
    map_then_unmap(16, a, m, 0x999);
    map_then_unmap(18, a, m, PROT_RO);
    other = a + AREA < 0xC0000000ULL ? a + AREA : a - AREA;
    put(46, other);
    map_then_unmap(20, other, m, PROT_RW);

    /* 337. */
    search_then_unmap(24, a, m);
    search_then_unmap(27, a + K64, m);
    rc = sc(SYS_MMAPPER_SEARCH_AND_MAP, a, m, PROT_RW, 0);
    put(30, rc);
    put(31, rc == 0 ? unmap(a) : SKIP);
    w32 = 0xFFFFFFFFu;
    x1 = sc(SYS_MMAPPER_SEARCH_AND_MAP, a, BAD_ID, PROT_RW, EA(&w32));
    put(32, x1);
    if (x1 == 0)
        unmap(w32);
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_SEARCH_AND_MAP, a, m1, PROT_RW, EA(&w32));
    x1 = w32;
    put(33, rc);
    put(34, x1);
    w32 = 0xFFFFFFFFu;
    rc2 = sc(SYS_MMAPPER_SEARCH_AND_MAP, a, m1, PROT_RW, EA(&w32));
    x2 = w32;
    put(35, rc2);
    put(36, x2);
    put(37, rc == 0 ? unmap(x1) : SKIP);
    put(38, rc2 == 0 ? unmap(x2) : SKIP);

    /* 335 with a NULL id pointer. */
    rc = sc(SYS_MMAPPER_MAP_SHM, a, m, PROT_RW, 0);
    put(40, rc);
    if (rc == 0) {
        put(41, sc(SYS_MMAPPER_UNMAP_SHM, a, 0, 0, 0));
        put(42, unmap(a));
    }

    put(43, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
    put(44, sc(SYS_MMAPPER_FREE_SHM, m1, 0, 0, 0));
    put(45, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));

done:
    write_frame();
    return 0;
}
