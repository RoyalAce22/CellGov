/* PPU probe: process queries, object counts, the stack information call,
 * the ids five creates return, and the read-only ss access control
 * subcommands.
 *
 * Calls, in order:
 *   sc 1 getpid, sc 18 getppid, sc 4 get_status of self, of the parent
 *   and of 0xDEAD, sc 31 get_ppu_guid, sc 49 get_stack_information;
 *   sc 12 get_number_of_object for every class in CLASSES;
 *   sc 169 spu_initialize, then one each of sc 100 mutex, sc 90
 *   semaphore, sc 82 event flag, sc 128 event queue and sc 170 SPU
 *   thread group; sc 12 again for every class; the five destroys
 *   (101, 91, 83, 129, 171);
 *   sc 871 ss_access_control_engine with pkg 2, 0 and 4 only.
 *
 * lv2_process_stack_small's main.c differs from this copy in the
 * STACK_SIZE line only.
 *
 * Payload, big-endian u64 words at the OFF_* offsets:
 *   OFF_QUERY  getpid, getppid, get_status(self), get_status(ppid),
 *              get_status(0xDEAD), get_ppu_guid, get_stack_information
 *              rc, the 8 bytes it wrote (u32 address, u32 size), r1 at
 *              main, STACK_SIZE
 *   OFF_COUNT  per class in CLASSES order: class, rc before, the 8
 *              bytes written before (pre-filled with 0xEE), rc after the
 *              creates, the 8 bytes written after
 *   OFF_CREATE spu_initialize rc, then per create (mutex, semaphore,
 *              event flag, event queue, SPU thread group): rc, id
 *   OFF_DESTROY the five destroy rcs, same order
 *   OFF_SS     per pkg (2, 0, 4): rc, then the 16 bytes at the a2
 *              buffer and the 16 bytes at the a3 buffer, both
 *              pre-filled with 0xEE
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define STACK_SIZE 0x800

SYS_PROCESS_PARAM(1001, STACK_SIZE)

#define SYS_PROCESS_GETPID          1
#define SYS_PROCESS_GET_STATUS      4
#define SYS_PROCESS_GET_NUM_OBJECT  12
#define SYS_PROCESS_GETPPID         18
#define SYS_PROCESS_GET_PPU_GUID    31
#define SYS_PPU_THREAD_GET_STACK    49
#define SYS_EVENT_FLAG_CREATE       82
#define SYS_EVENT_FLAG_DESTROY      83
#define SYS_SEMAPHORE_CREATE        90
#define SYS_SEMAPHORE_DESTROY       91
#define SYS_MUTEX_CREATE            100
#define SYS_MUTEX_DESTROY           101
#define SYS_EVENT_QUEUE_CREATE      128
#define SYS_EVENT_QUEUE_DESTROY     129
#define SYS_SPU_INITIALIZE          169
#define SYS_SPU_THREAD_GROUP_CREATE 170
#define SYS_SPU_THREAD_GROUP_DESTROY 171
#define SYS_SS_ACCESS_CONTROL       871

/* Every defined object class, then two undefined ones. */
static const unsigned int CLASSES[] = {
    0x08, 0x0A, 0x0B, 0x0E, 0x11, 0x21, 0x22, 0x23, 0x24, 0x25, 0x60,
    0x73, 0x85, 0x86, 0x88, 0x8D, 0x95, 0x96, 0x97, 0x98, 0x01, 0xFF,
};
#define NCLASSES (sizeof(CLASSES) / sizeof(CLASSES[0]))
#define NCREATES 5
static const unsigned int SS_PKGS[] = { 2, 0, 4 };
#define NSS 3

#define OFF_QUERY   0
#define QUERY_WORDS 10
#define OFF_COUNT   (OFF_QUERY + QUERY_WORDS * 8)
#define COUNT_WORDS 5
#define OFF_CREATE  (OFF_COUNT + NCLASSES * COUNT_WORDS * 8)
#define OFF_DESTROY (OFF_CREATE + (1 + 2 * NCREATES) * 8)
#define OFF_SS      (OFF_DESTROY + NCREATES * 8)
#define SS_WORDS    5
#define PAYLOAD     (OFF_SS + NSS * SS_WORDS * 8)

/* Attribute blocks, each field its own 32-bit word, as the probes that
 * created these objects on the console before used them. */
static const unsigned int sem_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0, 0, 0 };
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };
static const unsigned int mutex_attr[10] __attribute__((aligned(8))) =
    { 1, 0x20, 0x200, 0x2000, 0, 0, 0, 0, 0, 0 };
static const unsigned int queue_attr[4] __attribute__((aligned(8))) =
    { 1, 1, 0, 0 };
/* sys_spu_thread_group_attribute: nsize, name, type, option. */
static unsigned int group_attr[4] __attribute__((aligned(16)));
static const char GROUP_NAME[] = "queries";

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned char buf8[8] __attribute__((aligned(8)));
static volatile unsigned char ss_a2[16] __attribute__((aligned(16)));
static volatile unsigned char ss_a3[16] __attribute__((aligned(16)));
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

static void counts(unsigned word)
{
    unsigned i;
    for (i = 0; i < NCLASSES; i++) {
        unsigned off = OFF_COUNT + (i * COUNT_WORDS + word) * 8;
        fill(buf8, 8);
        put64(OFF_COUNT + i * COUNT_WORDS * 8, CLASSES[i]);
        put64(off, sc(SYS_PROCESS_GET_NUM_OBJECT, CLASSES[i], EA(buf8), 0, 0));
        put64(off + 8, raw8(buf8));
    }
}

int main(void)
{
    u64 r1, pid, ppid, rc;
    u64 ids[NCREATES];
    unsigned i;

    __asm__ volatile ("mr %0, 1" : "=r"(r1));
    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    pid = sc(SYS_PROCESS_GETPID, 0, 0, 0, 0);
    ppid = sc(SYS_PROCESS_GETPPID, 0, 0, 0, 0);
    put64(OFF_QUERY, pid);
    put64(OFF_QUERY + 8, ppid);
    put64(OFF_QUERY + 16, sc(SYS_PROCESS_GET_STATUS, pid, 0, 0, 0));
    put64(OFF_QUERY + 24, sc(SYS_PROCESS_GET_STATUS, ppid, 0, 0, 0));
    put64(OFF_QUERY + 32, sc(SYS_PROCESS_GET_STATUS, 0xDEAD, 0, 0, 0));
    put64(OFF_QUERY + 40, sc(SYS_PROCESS_GET_PPU_GUID, 0, 0, 0, 0));
    fill(buf8, 8);
    put64(OFF_QUERY + 48, sc(SYS_PPU_THREAD_GET_STACK, EA(buf8), 0, 0, 0));
    put64(OFF_QUERY + 56, raw8(buf8));
    put64(OFF_QUERY + 64, r1);
    put64(OFF_QUERY + 72, STACK_SIZE);

    counts(1);

    put64(OFF_CREATE, sc(SYS_SPU_INITIALIZE, 6, 0, 0, 0));
    for (i = 0; i < NCREATES; i++) {
        id_word = 0xFFFFFFFFu;
        switch (i) {
        case 0: rc = sc(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr), 0, 0); break;
        case 1: rc = sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1); break;
        case 2: rc = sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0, 0); break;
        case 3: rc = sc(SYS_EVENT_QUEUE_CREATE, EA(&id_word), EA(queue_attr), 0, 16); break;
        default:
            group_attr[0] = sizeof(GROUP_NAME);
            group_attr[1] = (unsigned int)(unsigned long)GROUP_NAME;
            group_attr[2] = 0;
            group_attr[3] = 0;
            rc = sc(SYS_SPU_THREAD_GROUP_CREATE, EA(&id_word), 1, 100, EA(group_attr));
            break;
        }
        ids[i] = rc == 0 ? id_word : 0;
        put64(OFF_CREATE + (1 + 2 * i) * 8, rc);
        put64(OFF_CREATE + (2 + 2 * i) * 8, id_word);
    }

    counts(3);

    for (i = 0; i < NCREATES; i++) {
        static const unsigned int destroy[NCREATES] = {
            SYS_MUTEX_DESTROY, SYS_SEMAPHORE_DESTROY, SYS_EVENT_FLAG_DESTROY,
            SYS_EVENT_QUEUE_DESTROY, SYS_SPU_THREAD_GROUP_DESTROY,
        };
        put64(OFF_DESTROY + i * 8, ids[i] ? sc(destroy[i], ids[i], 0, 0, 0) : 0xEEEEEEEEu);
    }

    for (i = 0; i < NSS; i++) {
        unsigned off = OFF_SS + i * SS_WORDS * 8;
        fill(ss_a2, 16);
        fill(ss_a3, 16);
        put64(off, sc(SYS_SS_ACCESS_CONTROL, SS_PKGS[i], EA(ss_a2), EA(ss_a3), 0));
        put64(off + 8, raw8(ss_a2));
        put64(off + 16, raw8(ss_a2 + 8));
        put64(off + 24, raw8(ss_a3));
        put64(off + 32, raw8(ss_a3 + 8));
    }

    write_frame();
    return 0;
}
