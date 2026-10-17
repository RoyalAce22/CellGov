/* PPU probe: whether a shared-memory segment keeps its contents across
 * sys_mmapper_unmap_shared_memory (335) and a later map of it (334, 337)
 * in a game process.
 *
 * The probe allocates one 256 MiB address area A of 64 KiB pages (330)
 * and one 64 KiB segment M (332), writes a word through its first
 * window, unmaps it, maps M again at another address and through a
 * search, and reads the word back each time. It frees both at the end.
 *
 * Payload: big-endian u64 words, one per slot. A read slot holds the
 * u32 at offset 0x10 of the window, or SKIP when the map before it
 * failed. When the setup in slots 0-3 fails, every later slot holds
 * SKIP.
 *    0  330 area rc                     1  A
 *    2  332 64K segment rc              3  M
 *    4  334 M at A
 *    5  the word at A+0x10 before any write
 *    6  335 at A, after writing MARK at A+0x10: rc
 *    7  334 M at A+128K                 8  the word at A+128K+0x10
 *    9  335 at A+128K: rc
 *   10  337 from A, M: rc              11  the address
 *   12  the word at that address+0x10  13  335 at that address: rc
 *   14  329 free M                     15  331 free A
 *   16  zero
 *
 * MARK is 0x12345678. SKIP is 0xDEADD00D.
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
#define PROT_RW   0x40000
#define AREA      0x10000000ULL
#define K64       0x10000ULL
#define MARK      0x12345678u
#define SKIP      0xDEADD00DULL

#define WORDS   17
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

static volatile unsigned int *word_at(u64 window)
{
    return (volatile unsigned int *)(unsigned long)(window + 0x10);
}

int main(void)
{
    unsigned i;
    u64 rc, a, m, x;

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
        put(15, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));
        goto done;
    }

    rc = sc(SYS_MMAPPER_MAP_SHM, a, m, PROT_RW, 0);
    put(4, rc);
    if (rc == 0) {
        put(5, *word_at(a));
        *word_at(a) = MARK;
        put(6, unmap(a));
    }

    rc = sc(SYS_MMAPPER_MAP_SHM, a + 2 * K64, m, PROT_RW, 0);
    put(7, rc);
    if (rc == 0) {
        put(8, *word_at(a + 2 * K64));
        put(9, unmap(a + 2 * K64));
    }

    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_SEARCH_AND_MAP, a, m, PROT_RW, EA(&w32));
    x = w32;
    put(10, rc);
    put(11, x);
    if (rc == 0) {
        put(12, *word_at(x));
        put(13, unmap(x));
    }

    put(14, sc(SYS_MMAPPER_FREE_SHM, m, 0, 0, 0));
    put(15, sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0));

done:
    write_frame();
    return 0;
}
