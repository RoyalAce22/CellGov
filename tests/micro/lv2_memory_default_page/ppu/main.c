/* PPU program: where sys_memory_allocate (348) places a block whose
 * flags name no page size, beside a block of 1 MiB pages.
 *
 * Slots, each a record of (u64 r3, u64 address; 0 on failure):
 *    0  N = 1 MiB, flags 0
 *    1  P = 1 MiB of 1 MiB pages      (next to N in one area?)
 *    2  free N (349)   3  free P (349)
 *
 * Payload: 4 records, 64 bytes. A slot never sent records
 * r3 = SKIP (0xDEADD00D).
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MEMORY_ALLOCATE          348
#define SYS_MEMORY_FREE              349

#define PAGE_1M   0x400
#define MIB       0x100000ULL
#define SKIP      0xDEADD00DULL

#define RECORDS 4
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

static void release(unsigned slot, u64 num, u64 addr)
{
    if (addr != 0)
        rec(slot, sc(num, addr, 0, 0, 0), addr);
}

int main(void)
{
    u64 n, p;
    unsigned i;

    for (i = 0; i < RECORDS; i++)
        rec(i, SKIP, 0);

    n = alloc(0, MIB, 0);
    p = alloc(1, MIB, PAGE_1M);
    release(2, SYS_MEMORY_FREE, n);
    release(3, SYS_MEMORY_FREE, p);
    write_frame();
    return 0;
}
