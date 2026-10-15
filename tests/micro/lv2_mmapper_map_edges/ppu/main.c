/* PPU probe: mmapper edge answers a game process gets in an area of
 * 64 KiB pages: a map that runs past the area's end (334), where a
 * search places a 1 MiB segment behind a 64 KiB map (337), the area
 * free at a non-base address and twice (331), and whether a freed
 * area's base is handed out again (330).
 *
 * The probe allocates one 256 MiB address area A (330), one 64 KiB
 * segment M and one 1 MiB segment M1 (332). Every map that succeeds is
 * unmapped (335); every object is freed before the frame is written.
 *
 * Payload: big-endian u64 words, one per slot. When the setup in slots
 * 0-5 fails, every later slot holds SKIP.
 *    0  330 area rc                     1  A
 *    2  332 64K segment rc              3  M
 *    4  332 1M segment rc               5  M1
 *    6  334 M1 at A + 256M - 64K (runs past the end of A)
 *    7  unmap of slot 6, or SKIP
 *    8  334 M at A
 *    9  337 M1 from A, behind M: rc   10  the address
 *   11  unmap of slot 10, or SKIP
 *   12  unmap of slot 8, or SKIP
 *   13  331 at A + 64K (not an area base)
 *   14  329 free M                     15  329 free M1
 *   16  331 free A                     17  331 free A again
 *   18  330 a second area rc           19  its base
 *   20  331 free of the second area, or SKIP
 *   21-22  SKIP (unused)
 *   23  zero
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

#define WORDS   24
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

int main(void)
{
    unsigned i;
    u64 rc, rc2, a, b, m, m1, x;

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
        put(16, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        goto done;
    }
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, MIB, FLAG_1M, EA(&w32));
    m1 = w32;
    put(4, rc);
    put(5, m1);
    if (rc != 0) {
        put(14, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
        put(16, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        goto done;
    }

    /* 334 past the end of A. */
    rc = sc(SYS_MMAPPER_MAP_SHM, a + AREA - K64, m1, PROT_RW, 0);
    put(6, rc);
    put(7, rc == 0 ? unmap(a + AREA - K64) : SKIP);

    /* 337 for M1 behind M at A. */
    rc = sc(SYS_MMAPPER_MAP_SHM, a, m, PROT_RW, 0);
    put(8, rc);
    w32 = 0xFFFFFFFFu;
    rc2 = sc(SYS_MMAPPER_SEARCH_AND_MAP, a, m1, PROT_RW, EA(&w32));
    x = w32;
    put(9, rc2);
    put(10, x);
    put(11, rc2 == 0 ? unmap(x) : SKIP);
    put(12, rc == 0 ? unmap(a) : SKIP);

    /* 331 at a non-base address, the frees, and 331 twice. */
    put(13, sc(SYS_MMAPPER_FREE_ADDRESS, a + K64, 0, 0, 0));
    put(14, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
    put(15, sc(SYS_MMAPPER_FREE_SHM, m1, 0, 0, 0));
    put(16, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
    put(17, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));

    /* 330 after the free. */
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_64K, AREA, EA(&w32));
    b = w32;
    put(18, rc);
    put(19, b);
    put(20, rc == 0 ? sc(SYS_MMAPPER_FREE_ADDRESS, b, 0, 0, 0) : SKIP);

done:
    write_frame();
    return 0;
}
