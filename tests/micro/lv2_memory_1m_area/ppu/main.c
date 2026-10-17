/* PPU program: where sys_memory_allocate (348) places blocks of 1 MiB
 * pages, and how they share the area range with
 * sys_mmapper_allocate_address (330).
 *
 * Slots, each a record of (u64 r3, u64 address; 0 on failure):
 *    0  A = 1 MiB of 1 MiB pages
 *    1  B = 1 MiB of 1 MiB pages
 *    2  C = 2 MiB of 1 MiB pages
 *    3  free B (349)
 *    4  D = 1 MiB of 1 MiB pages      (does it reuse B?)
 *    5  E = 64 KiB of 64 KiB pages
 *    6  M1 = a 256 MiB area (330)
 *    7  free A    8  free C    9  free D
 *   10  M2 = a 256 MiB area (330)     (does the 1 MiB-page granule return?)
 *   11  F = 1 MiB of 1 MiB pages
 *   12  free F   13  free E   14  free M2 (331)   15  free M1 (331)
 *
 * Payload: 16 records, 256 bytes. A slot never sent records
 * r3 = SKIP (0xDEADD00D).
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MEMORY_ALLOCATE          348
#define SYS_MEMORY_FREE              349
#define SYS_MMAPPER_ALLOCATE_ADDRESS 330
#define SYS_MMAPPER_FREE_ADDRESS     331

#define PAGE_64K  0x200
#define PAGE_1M   0x400
#define FLAG_1M   0x400
#define AREA      0x10000000ULL
#define MIB       0x100000ULL
#define K64       0x10000ULL
#define SKIP      0xDEADD00DULL

#define RECORDS 16
#define PAYLOAD (RECORDS * 16)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static void put64(unsigned off, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[off + i] = (unsigned char)(v >> (56 - 8 * i));
}

static void rec(unsigned slot, u64 rc, u64 addr)
{
    put64(slot * 16, rc);
    put64(slot * 16 + 8, addr);
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

/* One 348; records it and returns the address (0 on failure). */
static u64 alloc(unsigned slot, u64 size, u64 page)
{
    u64 rc;
    w32 = 0;
    rc = sc(SYS_MEMORY_ALLOCATE, size, page, EA(&w32), 0);
    rec(slot, rc, rc == 0 ? w32 : 0);
    return rc == 0 ? w32 : 0;
}

/* One 330; records it and returns the base (0 on failure). */
static u64 area(unsigned slot)
{
    u64 rc;
    w32 = 0;
    rc = sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_1M, AREA, EA(&w32));
    rec(slot, rc, rc == 0 ? w32 : 0);
    return rc == 0 ? w32 : 0;
}

static void release(unsigned slot, u64 num, u64 addr)
{
    if (addr != 0)
        rec(slot, sc(num, addr, 0, 0, 0), addr);
}

int main(void)
{
    u64 a, b, c, d, e, m1, m2, f;
    unsigned i;

    for (i = 0; i < RECORDS; i++)
        rec(i, SKIP, 0);

    a = alloc(0, MIB, PAGE_1M);
    b = alloc(1, MIB, PAGE_1M);
    c = alloc(2, 2 * MIB, PAGE_1M);
    release(3, SYS_MEMORY_FREE, b);
    d = alloc(4, MIB, PAGE_1M);
    e = alloc(5, K64, PAGE_64K);
    m1 = area(6);
    release(7, SYS_MEMORY_FREE, a);
    release(8, SYS_MEMORY_FREE, c);
    release(9, SYS_MEMORY_FREE, d);
    m2 = area(10);
    f = alloc(11, MIB, PAGE_1M);
    release(12, SYS_MEMORY_FREE, f);
    release(13, SYS_MEMORY_FREE, e);
    release(14, SYS_MMAPPER_FREE_ADDRESS, m2);
    release(15, SYS_MMAPPER_FREE_ADDRESS, m1);
    write_frame();
    return 0;
}
