/* PPU probe: the sys_mmapper_unmap_shared_memory (335) answers a game
 * process gets, and the free answers (329, 331) that depend on what is
 * still mapped.
 *
 * The probe allocates one 256 MiB address area A of 64 KiB pages (330),
 * two 64 KiB segments M and M2 and one 1 MiB segment M1 (332), and frees
 * them at the end.
 *
 * Payload: big-endian u64 words, one per slot. An "id" slot holds the
 * word 335 wrote to its out pointer, preset to 0xFFFFFFFF. When the
 * setup in slots 0-7 fails, every later slot holds SKIP.
 *    0  330 area rc                     1  A
 *    2  332 64K segment rc              3  M
 *    4  332 1M segment rc               5  M1
 *    6  332 64K segment rc              7  M2
 *    8  334 M1 at A (a 1 MiB window)
 *    9  335 at A+64K, inside the window: rc     10  id
 *   11  335 at A: rc                            12  id
 *   13  335 at A+256M or A-256M, outside every area: rc
 *   14  335 at A, nothing mapped, NULL id pointer: rc
 *   15  334 M at A                     16  334 M at A+128K
 *   17  335 at A: rc                   18  id
 *   19  329 free M, its window at A+128K still mapped
 *   20  335 at A+128K: rc              21  id
 *   22  329 free M, nothing mapped
 *   23  337 from A, M2, NULL out pointer
 *   24  335 at A after that search: rc
 *   25  329 free M2 after that search
 *   26  334 M2 at A                    27  335 at A: rc
 *   28  329 free M2 after a map and an unmap
 *   29  331 free A
 *   30  329 free M1
 *   31  329 free M2 again
 *   32  the id word after slot 13's refused unmap
 *   33  zero
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
#define AREA      0x10000000ULL
#define MIB       0x100000ULL
#define K64       0x10000ULL
#define SKIP      0xDEADD00DULL

#define WORDS   34
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

/* A 332 of `size` under `flags`: rc in `slot`, the id in `slot + 1`. */
static u64 alloc_shm(unsigned slot, u64 size, u64 flags, u64 *id)
{
    u64 rc;
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, size, flags, EA(&w32));
    *id = w32;
    put(slot, rc);
    put(slot + 1, *id);
    return rc;
}

int main(void)
{
    unsigned i;
    u64 rc, a, m, m1, m2, other;

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
    if (alloc_shm(2, K64, FLAG_64K, &m) != 0) {
        put(29, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        goto done;
    }
    if (alloc_shm(4, MIB, FLAG_1M, &m1) != 0) {
        put(22, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
        put(29, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        goto done;
    }
    if (alloc_shm(6, K64, FLAG_64K, &m2) != 0) {
        put(22, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
        put(30, sc(SYS_MMAPPER_FREE_SHM, m1, 0, 0, 0));
        put(29, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        goto done;
    }

    /* A 1 MiB window, unmapped from inside it and from its base. */
    rc = sc(SYS_MMAPPER_MAP_SHM, a, m1, PROT_RW, 0);
    put(8, rc);
    if (rc == 0) {
        put(9, unmap(a + K64));
        put(10, w32);
        put(11, unmap(a));
        put(12, w32);
    }

    /* Refusals with nothing mapped. */
    other = a + AREA < 0xC0000000ULL ? a + AREA : a - AREA;
    put(13, unmap(other));
    put(32, w32);
    put(14, sc(SYS_MMAPPER_UNMAP_SHM, a, 0, 0, 0));

    /* One segment mapped twice. */
    put(15, sc(SYS_MMAPPER_MAP_SHM, a, m, PROT_RW, 0));
    put(16, sc(SYS_MMAPPER_MAP_SHM, a + 2 * K64, m, PROT_RW, 0));
    put(17, unmap(a));
    put(18, w32);
    put(19, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
    put(20, unmap(a + 2 * K64));
    put(21, w32);
    put(22, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));

    /* A search that faults on its out pointer. */
    put(23, sc(SYS_MMAPPER_SEARCH_AND_MAP, a, m2, PROT_RW, 0));
    put(24, unmap(a));
    put(25, sc(SYS_MMAPPER_FREE_SHM, m2, 0, 0, 0));
    rc = sc(SYS_MMAPPER_MAP_SHM, a, m2, PROT_RW, 0);
    put(26, rc);
    if (rc == 0)
        put(27, unmap(a));
    put(28, sc(SYS_MMAPPER_FREE_SHM, m2, 0, 0, 0));

    put(29, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
    put(30, sc(SYS_MMAPPER_FREE_SHM, m1, 0, 0, 0));
    put(31, sc(SYS_MMAPPER_FREE_SHM, m2, 0, 0, 0));

done:
    write_frame();
    return 0;
}
