/* PPU probe: system calls whose guest buffers run across the seam
 * between two 64 KiB shared-memory segments mapped end to end.
 *
 * The probe allocates one 256 MiB area A of 64 KiB pages (330) and two
 * keyless 64 KiB segments M0 and M1 (332), and maps M0 at A and M1 at
 * A + 64K (334). E is A + 64K, the seam. Then, on the main thread:
 *
 *   mutex   sys_mutex_create (100) with its 0x28-byte attribute at E - 8:
 *           protocol and recursive sit in M0, pshared and the rest in M1.
 *           Once with pshared NOT_PROCESS_SHARED, once with pshared
 *           0x12345, an enumerant no mutex has. Each created mutex is
 *           destroyed (101).
 *   path    sys_spu_image_open (156) of an absent file, once with the
 *           path in the probe's own data and once with the path at
 *           E - 12, so that 12 bytes sit in M0 and the rest in M1. An
 *           open that succeeds is closed (158).
 *   out     sys_spu_initialize (169) with one Raw SPU, sys_raw_spu_create
 *           (160), then sys_raw_spu_get_int_mask (152) of class 2 three
 *           times: into the probe's own aligned data, into E - 20 (a
 *           4-byte-aligned doubleword inside M0), and into E - 4 (four
 *           bytes in M0, four in M1). The words there are 0xA5A5A5A5
 *           before each call. The Raw SPU is destroyed (161) and never
 *           started.
 *
 * Every map is unmapped and every object freed before the frame is
 * written.
 *
 * Payload: big-endian u32 words.
 *    0  DONE ("SEAM") once the main thread finished, PROG in a
 *       progress record
 *    1  the step a progress record names (0 otherwise): 1 the setup,
 *       2 the mutex cases, 3 the path cases, 4 the out cases, 5 the
 *       teardown
 *    2  330 rc          3  A (low word)
 *    4  332 M0 rc       5  332 M1 rc
 *    6  334 M0 at A     7  334 M1 at A + 64K
 *    8  100 rc, attribute across E      9  its 101 rc
 *   10  100 rc, pshared 0x12345 in M1  11  its 101 rc
 *   12  156 rc, path in the probe's data
 *   13  156 rc, path across E
 *   14  169 rc         15  160 rc
 *   16  152 rc, aligned out    17..18  the doubleword there
 *   19  152 rc, out at E - 20  20..21  the words at E - 20, E - 16
 *   22  152 rc, out at E - 4   23..24  the words at E - 4, E
 *   25  161 rc
 *   26  335 A + 64K    27  335 A
 *   28  329 M1         29  329 M0
 *   30  331 A
 *   31  zero
 *
 * SKIP is 0xDEADD00D. A step that does not run leaves its words SKIP.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MUTEX_CREATE             100
#define SYS_MUTEX_DESTROY            101
#define SYS_RAW_SPU_GET_INT_MASK     152
#define SYS_SPU_IMAGE_OPEN           156
#define SYS_SPU_IMAGE_CLOSE          158
#define SYS_RAW_SPU_CREATE           160
#define SYS_RAW_SPU_DESTROY          161
#define SYS_SPU_INITIALIZE           169
#define SYS_MMAPPER_FREE_SHM         329
#define SYS_MMAPPER_ALLOCATE_ADDRESS 330
#define SYS_MMAPPER_FREE_ADDRESS     331
#define SYS_MMAPPER_ALLOCATE_SHM     332
#define SYS_MMAPPER_MAP_SHM          334
#define SYS_MMAPPER_UNMAP_SHM        335

#define KEYLESS   0xFFFF000000000000ULL
#define FLAG_64K  0x200
#define PROT_RW   0x40000
#define AREA      0x10000000ULL
#define K64       0x10000ULL

#define SYNC_PRIORITY           0x2
#define SYNC_NOT_RECURSIVE      0x20
#define SYNC_NOT_PROCESS_SHARED 0x200
#define SYNC_NOT_ADAPTIVE       0x2000
#define PSHARED_UNKNOWN         0x12345
#define MUTEX_ATTR_BYTES        0x28

#define INT_CLASS_2 2
#define FILL        0xA5A5A5A5u

#define WORDS    32
#define DONE     0x5345414Du /* "SEAM" */
#define PROGRESS 0x50524F47u /* "PROG" */
#define SKIP     0xDEADD00Du

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };
static const char ABSENT[] = "/dev_hdd0/tmp/cgov_span_absent.elf";

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile unsigned long long mask __attribute__((aligned(8)));
static unsigned char image[16] __attribute__((aligned(16)));
static char path_here[sizeof(ABSENT)] __attribute__((aligned(16)));

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static void write_file(void)
{
    CGOV_OUT_FILE_WRITE((const void *)result, sizeof(result));
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    write_file();
}

static void progress(unsigned int step)
{
    result[0] = PROGRESS;
    result[1] = step;
    write_file();
}

static volatile unsigned int *word_at(u64 addr)
{
    return (volatile unsigned int *)(unsigned long)addr;
}

/* The mutex attribute at `at`, as aligned word stores. */
static void put_mutex_attr(u64 at, unsigned int pshared)
{
    unsigned int i;
    for (i = 0; i < MUTEX_ATTR_BYTES / 4; i++)
        word_at(at)[i] = 0;
    word_at(at)[0] = SYNC_PRIORITY;
    word_at(at)[1] = SYNC_NOT_RECURSIVE;
    word_at(at)[2] = pshared;
    word_at(at)[3] = SYNC_NOT_ADAPTIVE;
    /* name "seam" */
    word_at(at)[7] = 0x7365616Du;
}

/* One create with the attribute at `at`, and the destroy it owes. */
static void mutex_case(u64 at, unsigned int pshared, unsigned int rc_word)
{
    put_mutex_attr(at, pshared);
    w32 = 0xFFFFFFFFu;
    result[rc_word] = (unsigned int)sc(SYS_MUTEX_CREATE, EA(&w32), at, 0, 0);
    if (result[rc_word] == 0)
        result[rc_word + 1] = (unsigned int)sc(SYS_MUTEX_DESTROY, w32, 0, 0, 0);
}

static void path_case(char *dst, unsigned int rc_word)
{
    unsigned int i;
    for (i = 0; i < sizeof(ABSENT); i++)
        dst[i] = ABSENT[i];
    result[rc_word] = (unsigned int)sc(SYS_SPU_IMAGE_OPEN, EA(image), EA(dst), 0, 0);
    if (result[rc_word] == 0)
        sc(SYS_SPU_IMAGE_CLOSE, EA(image), 0, 0, 0);
}

/* get_int_mask into `out`, recording the rc and the two words there. */
static void mask_case(unsigned int id, u64 out, unsigned int rc_word)
{
    word_at(out)[0] = FILL;
    word_at(out)[1] = FILL;
    result[rc_word] = (unsigned int)sc(SYS_RAW_SPU_GET_INT_MASK, id, INT_CLASS_2, out, 0);
    result[rc_word + 1] = word_at(out)[0];
    result[rc_word + 2] = word_at(out)[1];
}

static u64 unmap(u64 addr)
{
    w32 = 0xFFFFFFFFu;
    return sc(SYS_MMAPPER_UNMAP_SHM, addr, EA(&w32), 0, 0);
}

int main(void)
{
    unsigned int i, map0 = 0, map1 = 0, m0_ok = 0, m1_ok = 0, raw_ok, raw_id;
    u64 a = 0, m0 = 0, m1 = 0, e;

    for (i = 0; i < WORDS; i++)
        result[i] = i < 31 ? SKIP : 0;
    progress(1);

    w32 = 0xFFFFFFFFu;
    result[2] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_64K, AREA, EA(&w32));
    a = w32;
    result[3] = (unsigned int)a;
    if (result[2] != 0)
        goto done;
    w32 = 0xFFFFFFFFu;
    result[4] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, K64, FLAG_64K, EA(&w32));
    m0 = w32;
    m0_ok = result[4] == 0;
    w32 = 0xFFFFFFFFu;
    result[5] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, K64, FLAG_64K, EA(&w32));
    m1 = w32;
    m1_ok = result[5] == 0;
    map0 = map1 = 0;
    if (m0_ok && m1_ok) {
        result[6] = (unsigned int)sc(SYS_MMAPPER_MAP_SHM, a, m0, PROT_RW, 0);
        map0 = result[6] == 0;
        if (map0) {
            result[7] = (unsigned int)sc(SYS_MMAPPER_MAP_SHM, a + K64, m1, PROT_RW, 0);
            map1 = result[7] == 0;
        }
    }

    if (map0 && map1) {
        e = a + K64;

        progress(2);
        mutex_case(e - 8, SYNC_NOT_PROCESS_SHARED, 8);
        mutex_case(e - 8, PSHARED_UNKNOWN, 10);

        progress(3);
        path_case(path_here, 12);
        path_case((char *)(unsigned long)(e - 12), 13);

        progress(4);
        result[14] = (unsigned int)sc(SYS_SPU_INITIALIZE, 6, 1, 0, 0);
        w32 = 0xEEEEEEEEu;
        result[15] = (unsigned int)sc(SYS_RAW_SPU_CREATE, EA(&w32), 0, 0, 0);
        raw_id = w32;
        raw_ok = result[15] == 0;
        if (raw_ok) {
            mask_case(raw_id, EA(&mask), 16);
            mask_case(raw_id, e - 20, 19);
            mask_case(raw_id, e - 4, 22);
            result[25] = (unsigned int)sc(SYS_RAW_SPU_DESTROY, raw_id, 0, 0, 0);
        }
    }

    progress(5);
    if (map1)
        result[26] = (unsigned int)unmap(a + K64);
    if (map0)
        result[27] = (unsigned int)unmap(a);
    if (m1_ok)
        result[28] = (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m1, 0, 0, 0);
    if (m0_ok)
        result[29] = (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m0, 0, 0, 0);
    result[30] = (unsigned int)sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0);

done:
    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
