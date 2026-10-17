/* PPU probe: the process's pid and ppid, which object classes
 * sys_process_get_number_of_object (12) answers, and what a keyed
 * sys_mmapper_allocate_shared_memory (332) answers from a game process
 * for the cellSysutil segment key and for an unused key beside it.
 *
 * Calls, in order:
 *   sc 1 getpid, sc 18 getppid;
 *   sc 12 for every class 0x00..0xFF, its out word pre-filled with 0xEE;
 *   sc 332 on key 0x8006010000000010, then on 0x8006010000000FF0, each
 *   64 KiB with 64 KiB pages; a create that succeeds is freed (sc 329);
 *   sc 12 for the memory class (0x08) again.
 *
 * Payload, big-endian u64 words:
 *   +0    getpid, getppid
 *   +16   per 332 call (cellSysutil key, then the unused key): its
 *         return code, the id word (pre-filled 0xFFFFFFFF), the free's
 *         return code (0xFFFFFFFF when nothing was created)
 *   +64   the memory class count's return code and the 8 bytes it wrote
 *   +80   per class k: sc 12's return code, the 8 bytes it wrote
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PROCESS_GETPID          1
#define SYS_PROCESS_GET_NUM_OBJECT  12
#define SYS_PROCESS_GETPPID         18
#define SYS_MMAPPER_FREE_SHM        329
#define SYS_MMAPPER_ALLOCATE_SHM    332

#define SYSUTIL_SHM_KEY  0x8006010000000010ull
#define UNUSED_KEY       0x8006010000000FF0ull
#define SHM_SIZE         0x10000
#define PAGE_SIZE_64K    0x200
#define MEM_CLASS        0x08
#define NOT_RUN          0xFFFFFFFFull

#define OFF_SHM     16
#define OFF_MEM     64
#define OFF_CLASSES 80
#define NCLASSES    256
#define PAYLOAD     (OFF_CLASSES + NCLASSES * 16)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned char buf8[8] __attribute__((aligned(8)));
static volatile unsigned int id_word __attribute__((aligned(8)));

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

static u64 raw8(const volatile unsigned char *p)
{
    u64 v = 0;
    unsigned i;
    for (i = 0; i < 8; i++)
        v = v << 8 | p[i];
    return v;
}

static void fill(volatile unsigned char *p, unsigned n)
{
    unsigned i;
    for (i = 0; i < n; i++)
        p[i] = 0xEE;
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

/* One keyed 332 at `off`: its return code, the id word, and the free of
 * anything it created. */
static void shm(unsigned off, u64 key)
{
    u64 rc;
    id_word = 0xFFFFFFFFu;
    rc = sc(SYS_MMAPPER_ALLOCATE_SHM, key, SHM_SIZE, PAGE_SIZE_64K, EA(&id_word));
    put64(off, rc);
    put64(off + 8, id_word);
    put64(off + 16, (unsigned int)rc == 0 ? sc(SYS_MMAPPER_FREE_SHM, id_word, 0, 0, 0) : NOT_RUN);
}

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    put64(0, sc(SYS_PROCESS_GETPID, 0, 0, 0, 0));
    put64(8, sc(SYS_PROCESS_GETPPID, 0, 0, 0, 0));

    for (i = 0; i < NCLASSES; i++) {
        fill(buf8, 8);
        put64(OFF_CLASSES + i * 16, sc(SYS_PROCESS_GET_NUM_OBJECT, i, EA(buf8), 0, 0));
        put64(OFF_CLASSES + i * 16 + 8, raw8(buf8));
    }

    shm(OFF_SHM, SYSUTIL_SHM_KEY);
    shm(OFF_SHM + 24, UNUSED_KEY);

    fill(buf8, 8);
    put64(OFF_MEM, sc(SYS_PROCESS_GET_NUM_OBJECT, MEM_CLASS, EA(buf8), 0, 0));
    put64(OFF_MEM + 8, raw8(buf8));

    write_frame();
    return 0;
}
