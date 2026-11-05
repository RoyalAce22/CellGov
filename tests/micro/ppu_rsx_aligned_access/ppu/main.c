/* PPU probe: aligned loads and stores inside RSX local memory once
 * libgcm has brought up an RSX context.
 *
 * RSX local memory is caching-inhibited for a game process. The PPE
 * refuses some accesses there with an alignment interrupt; this probe
 * sends only accesses it expects to run. A 1 MiB-page host block and
 * rsxInit map local memory; its first 256 bytes are filled with a
 * pattern. Each access writes a progress record naming it before it
 * runs, so a run that stops names the access that stopped it; the
 * final frame replaces the record.
 *
 * Payload (11 big-endian u64):
 *   0       DONE once every access ran, PROG in a progress record
 *   1       the step a progress record names (0 in the final frame)
 *   2       rsxInit rc (0xFFFFFFFFFFFFFFFF when not sent)
 *   3       local memory address from gcmGetConfiguration
 *   4       step 1: lwz at local + 0x40
 *   5       step 2: ld at local + 0x48
 *   6       step 3: lhz at local + 0x52
 *   7       step 4: lfd at local + 0x58, as raw bits
 *   8       step 5: stw of 0x11223344 at local + 0x60, then ld of local + 0x60
 *   9       step 6: std of 0x5566778899AABBCC at local + 0x68, then ld of it
 *   10      step 7: lbz at local + 0x71
 *
 * The fill puts 0xA5A5A5A5_0000000i in u64 i of the 256 bytes.
 * SKIP is 0xDEADD00D.
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000

#define DONE 0x444F4E45ULL
#define PROG 0x50524F47ULL
#define SKIP 0xDEADD00DULL
#define FILL 0xA5A5A5A500000000ULL

#define FILL_WORDS 32
#define WORDS   11
#define PAYLOAD (WORDS * 8)

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static gcmContextData *context;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static void progress(u64 step)
{
    result[1] = step;
    CGOV_OUT_FILE_WRITE((const void *)result, PAYLOAD);
}

int main(void)
{
    sys_mem_addr_t host = 0;
    gcmConfiguration config;
    volatile u64 *local;
    unsigned long base;
    u64 v;
    unsigned i;

    result[0] = PROG;
    result[1] = 0;
    result[2] = ~0ULL;
    for (i = 3; i < WORDS; i++)
        result[i] = SKIP;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        write_frame();
        return 1;
    }
    result[2] = (u64)(s64)rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    if (result[2] != 0) {
        write_frame();
        return 1;
    }
    gcmGetConfiguration(&config);
    local = (volatile u64 *)config.localAddress;
    base = (unsigned long)local;
    result[3] = base;
    for (i = 0; i < FILL_WORDS; i++)
        local[i] = FILL | i;

    progress(1);
    __asm__ volatile("lwz %0,0x40(%1)" : "=r"(v) : "b"(base) : "memory");
    result[4] = v;

    progress(2);
    __asm__ volatile("ld %0,0x48(%1)" : "=r"(v) : "b"(base) : "memory");
    result[5] = v;

    progress(3);
    __asm__ volatile("lhz %0,0x52(%1)" : "=r"(v) : "b"(base) : "memory");
    result[6] = v;

    progress(4);
    {
        union { double d; u64 u; } bits;
        __asm__ volatile("lfd %0,0x58(%1)" : "=f"(bits.d) : "b"(base) : "memory");
        result[7] = bits.u;
    }

    progress(5);
    __asm__ volatile("stw %1,0x60(%2)\n\tld %0,0x60(%2)"
                     : "=&r"(v) : "r"(0x11223344u), "b"(base) : "memory");
    result[8] = v;

    progress(6);
    __asm__ volatile("std %1,0x68(%2)\n\tld %0,0x68(%2)"
                     : "=&r"(v) : "r"(0x5566778899AABBCCULL), "b"(base) : "memory");
    result[9] = v;

    progress(7);
    __asm__ volatile("lbz %0,0x71(%1)" : "=r"(v) : "b"(base) : "memory");
    result[10] = v;

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
