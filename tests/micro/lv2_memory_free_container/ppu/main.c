/* PPU probe: memory allocate, free and container answers a game process
 * gets: sys_memory_container_create (324, 341) and destroy (342),
 * sys_memory_allocate (348), sys_memory_free (349),
 * sys_memory_allocate_from_container (350) and
 * sys_memory_get_user_memory_size (352).
 *
 * At most two containers of 1 MiB and three 1 MiB allocations are live
 * at once, and the probe frees and destroys everything it takes.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 * "avail" is the available_user_memory word sc 352 writes.
 *   A. user memory
 *    0  sc 352(&info) rc          1  total            2  avail
 *   B. containers (cid words pre-filled 0xFFFFFFFF)
 *    3  sc 324(NULL, 0x80000) rc      4  sc 324(NULL, 0x100000) rc
 *    5  sc 324(&c, 0x80000) rc        6  sc 324(&c, 0x100000) rc
 *    7  that cid                      8  avail after it
 *    9  sc 342(that cid) rc, or SKIP  10 avail after it
 *   11  sc 341(NULL, 0x80000) rc     12  sc 341(NULL, 0x100000) rc
 *   13  sc 341(&c, 0x80000) rc       14  sc 341(&c, 0x180000) rc
 *   15  that cid                     16  avail after it
 *   17  sc 342(that cid) rc, or SKIP 18  avail after it
 *   19  sc 342(that cid) again rc    20  sc 342(0xDEADBEEF) rc
 *   C. sc 348 (address words pre-filled 0xFFFFFFFF)
 *   21  sc 348(0, 1M, &a) rc             22  a
 *   23  sc 348(0x1000, 1M, &a) rc        24  a
 *   25  sc 348(0x100000, 0x999, &a) rc   26  a
 *   27  avail before the null case
 *   28  sc 348(0x100000, 1M, NULL) rc    29  avail after it
 *   D. sc 349
 *   30  sc 348(0x100000, 1M, &a1) rc     31  a1
 *   32  avail after it
 *   33  sc 349(a1) rc                    34  avail after it
 *   35  sc 349(a1) again rc              36  sc 349(0xDEAD0000) rc
 *   37  sc 348(0x100000, 1M, &a2) rc     38  a2
 *   39  sc 349(a2 + 0x10000) rc          40  sc 349(a2) rc
 *   E. sc 350 against a 1 MiB container from 341
 *   41  sc 341(&c, 0x100000) rc          42  that cid
 *   43  sc 350(0, c, 1M, &a) rc
 *   44  sc 350(0x1000, c, 0x999, &a) rc
 *   45  sc 350(0x100000, 0xDEADBEEF, 0x999, &a) rc
 *   46  sc 350(0x100001, 0xDEADBEEF, 1M, &a) rc
 *   47  sc 350(0x200000, c, 1M, &a) rc
 *   48  sc 350(0x200000, c, 1M, NULL) rc
 *   49  sc 350(0x100000, c, 1M, NULL) rc
 *   50  sc 350(0x100000, c, 1M, &a3) rc  51  a3
 *   52  sc 342(c) with a3 live rc
 *   53  sc 349(a3) rc                    54  sc 342(c) rc
 *   55  avail at the end
 *
 * SKIP is 0xDEADD00D: the create did not succeed, so nothing was
 * destroyed. Every sc 349 and sc 342 the probe makes on an id or
 * address that holds memory is in a slot above.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MEMORY_CONTAINER_CREATE_OLD 324
#define SYS_MEMORY_CONTAINER_CREATE     341
#define SYS_MEMORY_CONTAINER_DESTROY    342
#define SYS_MEMORY_ALLOCATE             348
#define SYS_MEMORY_FREE                 349
#define SYS_MEMORY_ALLOCATE_FROM_CONT   350
#define SYS_MEMORY_GET_USER_MEMORY_SIZE 352

#define PAGE_1M 0x400
#define MIB     0x100000
#define SKIP    0xDEADD00DULL

#define WORDS   56
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile unsigned int info[2] __attribute__((aligned(8)));

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

static u64 avail(void)
{
    info[0] = info[1] = 0xFFFFFFFFu;
    (void)sc(SYS_MEMORY_GET_USER_MEMORY_SIZE, EA(info), 0, 0, 0);
    return info[1];
}

/* One create on `num`: rc in `slot`, and the cid in `slot + 1`.
 * Returns the cid when the create succeeded, else 0. */
static u64 create(unsigned num, u64 size, unsigned rc_slot, unsigned cid_slot)
{
    u64 rc;
    w32 = 0xFFFFFFFFu;
    rc = sc(num, EA(&w32), size, 0, 0);
    put(rc_slot, rc);
    if (cid_slot)
        put(cid_slot, w32);
    return rc == 0 ? w32 : 0;
}

static u64 allocate(u64 size, u64 flags, unsigned rc_slot, unsigned addr_slot)
{
    u64 rc;
    w32 = 0xFFFFFFFFu;
    rc = sc(SYS_MEMORY_ALLOCATE, size, flags, EA(&w32), 0);
    put(rc_slot, rc);
    put(addr_slot, w32);
    return rc == 0 ? w32 : 0;
}

int main(void)
{
    unsigned i;
    u64 c, a, a1, a2, a3;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    /* A. */
    info[0] = info[1] = 0xFFFFFFFFu;
    put(0, sc(SYS_MEMORY_GET_USER_MEMORY_SIZE, EA(info), 0, 0, 0));
    put(1, info[0]);
    put(2, info[1]);

    /* B. */
    put(3, sc(SYS_MEMORY_CONTAINER_CREATE_OLD, 0, 0x80000, 0, 0));
    put(4, sc(SYS_MEMORY_CONTAINER_CREATE_OLD, 0, MIB, 0, 0));
    c = create(SYS_MEMORY_CONTAINER_CREATE_OLD, 0x80000, 5, 0);
    if (c)
        (void)sc(SYS_MEMORY_CONTAINER_DESTROY, c, 0, 0, 0);
    c = create(SYS_MEMORY_CONTAINER_CREATE_OLD, MIB, 6, 7);
    put(8, avail());
    put(9, c ? sc(SYS_MEMORY_CONTAINER_DESTROY, c, 0, 0, 0) : SKIP);
    put(10, avail());

    put(11, sc(SYS_MEMORY_CONTAINER_CREATE, 0, 0x80000, 0, 0));
    put(12, sc(SYS_MEMORY_CONTAINER_CREATE, 0, MIB, 0, 0));
    c = create(SYS_MEMORY_CONTAINER_CREATE, 0x80000, 13, 0);
    if (c)
        (void)sc(SYS_MEMORY_CONTAINER_DESTROY, c, 0, 0, 0);
    c = create(SYS_MEMORY_CONTAINER_CREATE, 0x180000, 14, 15);
    put(16, avail());
    put(17, c ? sc(SYS_MEMORY_CONTAINER_DESTROY, c, 0, 0, 0) : SKIP);
    put(18, avail());
    put(19, sc(SYS_MEMORY_CONTAINER_DESTROY, c, 0, 0, 0));
    put(20, sc(SYS_MEMORY_CONTAINER_DESTROY, 0xDEADBEEFu, 0, 0, 0));

    /* C. */
    a = allocate(0, PAGE_1M, 21, 22);
    if (a)
        (void)sc(SYS_MEMORY_FREE, a, 0, 0, 0);
    a = allocate(0x1000, PAGE_1M, 23, 24);
    if (a)
        (void)sc(SYS_MEMORY_FREE, a, 0, 0, 0);
    a = allocate(MIB, 0x999, 25, 26);
    if (a)
        (void)sc(SYS_MEMORY_FREE, a, 0, 0, 0);
    put(27, avail());
    put(28, sc(SYS_MEMORY_ALLOCATE, MIB, PAGE_1M, 0, 0));
    put(29, avail());

    /* D. */
    a1 = allocate(MIB, PAGE_1M, 30, 31);
    put(32, avail());
    put(33, sc(SYS_MEMORY_FREE, a1, 0, 0, 0));
    put(34, avail());
    put(35, sc(SYS_MEMORY_FREE, a1, 0, 0, 0));
    put(36, sc(SYS_MEMORY_FREE, 0xDEAD0000u, 0, 0, 0));
    a2 = allocate(MIB, PAGE_1M, 37, 38);
    put(39, sc(SYS_MEMORY_FREE, a2 + 0x10000, 0, 0, 0));
    put(40, sc(SYS_MEMORY_FREE, a2, 0, 0, 0));

    /* E. */
    c = create(SYS_MEMORY_CONTAINER_CREATE, MIB, 41, 42);
    w32 = 0xFFFFFFFFu;
    put(43, sc(SYS_MEMORY_ALLOCATE_FROM_CONT, 0, c, PAGE_1M, EA(&w32)));
    put(44, sc(SYS_MEMORY_ALLOCATE_FROM_CONT, 0x1000, c, 0x999, EA(&w32)));
    put(45, sc(SYS_MEMORY_ALLOCATE_FROM_CONT, MIB, 0xDEADBEEFu, 0x999, EA(&w32)));
    put(46, sc(SYS_MEMORY_ALLOCATE_FROM_CONT, MIB + 1, 0xDEADBEEFu, PAGE_1M, EA(&w32)));
    put(47, sc(SYS_MEMORY_ALLOCATE_FROM_CONT, 2 * MIB, c, PAGE_1M, EA(&w32)));
    put(48, sc(SYS_MEMORY_ALLOCATE_FROM_CONT, 2 * MIB, c, PAGE_1M, 0));
    put(49, sc(SYS_MEMORY_ALLOCATE_FROM_CONT, MIB, c, PAGE_1M, 0));
    w32 = 0xFFFFFFFFu;
    put(50, sc(SYS_MEMORY_ALLOCATE_FROM_CONT, MIB, c, PAGE_1M, EA(&w32)));
    a3 = w32;
    put(51, a3);
    put(52, sc(SYS_MEMORY_CONTAINER_DESTROY, c, 0, 0, 0));
    put(53, sc(SYS_MEMORY_FREE, a3, 0, 0, 0));
    put(54, sc(SYS_MEMORY_CONTAINER_DESTROY, c, 0, 0, 0));
    put(55, avail());

    write_frame();
    return 0;
}
