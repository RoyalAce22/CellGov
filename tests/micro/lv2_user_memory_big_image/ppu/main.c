/* PPU probe: what sys_memory_get_user_memory_size (352) reports for a
 * game process, and how a memory container moves it. Built twice from
 * this source with different inputs: STACK_SIZE is the declared primary
 * stack, PAD_BYTES a zeroed array that grows the loaded image.
 *
 * Payload: big-endian u64 words.
 *    0  sc 352 rc at entry      1  total      2  available
 *    3  sc 341(&c, 1 MiB) rc    4  total      5  available
 *    6  sc 342(c) rc, or SKIP   7  total      8  available
 *    9  r1 in main             10  the address one past the pad array
 *   11  STACK_SIZE             12  PAD_BYTES
 *   13  0xC0DE0352, written last
 *
 * SKIP is 0xDEADD00D: the create failed, so nothing was destroyed.
 */

/* This copy's input. */
#define PAD_BYTES 0x800000

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#ifndef STACK_SIZE
#define STACK_SIZE 0x10000
#endif
#ifndef PAD_BYTES
#define PAD_BYTES 16
#endif

SYS_PROCESS_PARAM(1001, STACK_SIZE)

#define SYS_MEMORY_CONTAINER_CREATE     341
#define SYS_MEMORY_CONTAINER_DESTROY    342
#define SYS_MEMORY_GET_USER_MEMORY_SIZE 352

#define MIB     0x100000
#define SKIP    0xDEADD00DULL

#define WORDS   14
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile unsigned int info[2] __attribute__((aligned(8)));
static volatile unsigned char pad[PAD_BYTES] __attribute__((aligned(128)));

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

/* sc 352 into the three words from `slot`. */
static void sizes(unsigned slot)
{
    info[0] = info[1] = 0xFFFFFFFFu;
    put(slot, sc(SYS_MEMORY_GET_USER_MEMORY_SIZE, EA(info), 0, 0, 0));
    put(slot + 1, info[0]);
    put(slot + 2, info[1]);
}

int main(void)
{
    unsigned i;
    u64 r1, rc;

    __asm__ volatile ("mr %0, 1" : "=r"(r1));
    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    sizes(0);
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MEMORY_CONTAINER_CREATE, EA(&w32), MIB, 0, 0);
    info[0] = info[1] = 0xFFFFFFFFu;
    (void)sc(SYS_MEMORY_GET_USER_MEMORY_SIZE, EA(info), 0, 0, 0);
    put(3, rc);
    put(4, info[0]);
    put(5, info[1]);
    put(6, rc == 0 ? sc(SYS_MEMORY_CONTAINER_DESTROY, w32, 0, 0, 0) : SKIP);
    info[0] = info[1] = 0xFFFFFFFFu;
    (void)sc(SYS_MEMORY_GET_USER_MEMORY_SIZE, EA(info), 0, 0, 0);
    put(7, info[0]);
    put(8, info[1]);

    put(9, r1);
    put(10, EA(pad) + PAD_BYTES);
    put(11, STACK_SIZE);
    put(12, PAD_BYTES);
    put(13, 0xC0DE0352u);
    write_frame();
    return 0;
}
