/* PPU probe: where the system places a game process's TLS, its firmware
 * PRX segments and its first user-memory allocations, against the end
 * of the game's own image.
 *
 * IMAGE_PAD bytes of zero-filled data move the end of the image. The
 * test lv2_process_layout builds with no pad (the image ends near
 * 0x10020000); lv2_process_layout_wide builds with 6 MiB (the image
 * ends past 0x10400000). The two captures show whether each placement
 * follows the image.
 *
 * Calls: sc 351 (sys_memory_get_page_attribute) reads the attribute of
 * one page and changes nothing. The probe loads one firmware module
 * through the system module library, allocates two blocks with
 * sys_memory_allocate, then frees both blocks and unloads the module.
 *
 * Payload, big-endian, laid out by the OFF_* offsets:
 *   OFF_HDR     u64 r13 at main, u64 end of the pad (0 with no pad),
 *               u64 IMAGE_PAD, u64 address of the payload buffer,
 *               u64 address of main's OPD
 *   OFF_CALLS   u64 return codes and addresses, in the order the
 *               CALL_* indices list
 *   OFF_MODLIST u64 rc, u64 count of the module list after the load
 *   OFF_MODINFO MOD_MAX records of MOD_REC bytes: u32 id, u32 rc,
 *               u32 all_segments_num, u32 segments_num, 32 bytes of
 *               name, SEG_MAX x (u64 base, u64 memsize)
 *   OFF_MAP0..2 page-attribute sweeps at main, after the load and
 *               after the allocations: u64 run count, then MAP_RUNS x
 *               (u32 start, u32 end, u32 attribute, u32 page size)
 */

#include <sys/process.h>
#include <sys/tty.h>
#include <sysmodule/sysmodule.h>
/* sys/prx.h and lv2/prx.h share one include guard in PSL1GHT, so the
 * liblv2 prototypes are declared here. */
#include <sys/prx.h>
s32 sysPrxGetModuleList(sysPrxFlags flags, sysPrxModuleList *list);
s32 sysPrxGetModuleInfo(sysPrxId id, sysPrxFlags flags, sysPrxModuleInfo *info);

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

/* lv2_process_layout_wide's main.c differs from this copy in this line
 * only. */
#define IMAGE_PAD 0

#define SYS_MEMORY_ALLOCATE       348
#define SYS_MEMORY_FREE           349
#define SYS_MEMORY_GET_PAGE_ATTR  351

#define PAGE_64K 0x200
#define PAGE_1M  0x400

#define CALL_LOAD       0
#define CALL_ALLOC64K   1
#define CALL_ADDR64K    2
#define CALL_ALLOC1M    3
#define CALL_ADDR1M     4
#define CALL_FREE64K    5
#define CALL_FREE1M     6
#define CALL_UNLOAD     7
#define CALLS           8

#define MOD_MAX   12
#define SEG_MAX   4
#define MOD_REC   (16 + 32 + SEG_MAX * 16)
#define MAP_RUNS  40
#define MAP_REC   (8 + MAP_RUNS * 16)

#define OFF_HDR      0x000
#define OFF_CALLS    0x028
#define OFF_MODLIST  0x068
#define OFF_MODINFO  0x078
#define OFF_MAP0     (OFF_MODINFO + MOD_MAX * MOD_REC)
#define OFF_MAP1     (OFF_MAP0 + MAP_REC)
#define OFF_MAP2     (OFF_MAP1 + MAP_REC)
#define PAYLOAD      (OFF_MAP2 + MAP_REC)

#define SWEEP_FROM  0x10000ULL
#define SWEEP_TO    0xF0000000ULL
#define SWEEP_STEP  0x10000ULL

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned char scratch[128] __attribute__((aligned(128)));
static volatile u32 idbuf[64] __attribute__((aligned(8)));
static volatile u32 lvbuf[64] __attribute__((aligned(8)));
static volatile sysPrxSegmentInfo segbuf[SEG_MAX] __attribute__((aligned(8)));
static volatile u32 addr_word __attribute__((aligned(8)));
#if IMAGE_PAD
static volatile unsigned char image_pad[IMAGE_PAD] __attribute__((aligned(0x10000)));
#endif

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c)
{
    return (u64)cgov_sc(num, a, b, c, 0, 0, 0, 0, 0);
}

static void put32(unsigned off, u32 v)
{
    out[off] = v >> 24; out[off + 1] = v >> 16;
    out[off + 2] = v >> 8; out[off + 3] = v;
}

static void put64(unsigned off, u64 v)
{
    put32(off, (u32)(v >> 32));
    put32(off + 4, (u32)v);
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

/* Every mapped 64 KiB step from SWEEP_FROM to SWEEP_TO, merged into
 * runs of one attribute and one page size. */
static void sweep(unsigned off)
{
    u64 addr, run_start = 0, run_attr = 0, run_page = 0, nruns = 0;
    int in_run = 0;
    for (addr = SWEEP_FROM; addr <= SWEEP_TO; addr += SWEEP_STEP) {
        u64 rc = 1, attr = 0, page = 0;
        unsigned i;
        if (addr < SWEEP_TO) {
            for (i = 0; i < 32; i++)
                scratch[i] = 0;
            rc = sc(SYS_MEMORY_GET_PAGE_ATTR, addr, EA(scratch), 0);
            attr = *(volatile u64 *)scratch;
            page = *(volatile u32 *)(scratch + 16);
        }
        if (in_run && (rc != 0 || attr != run_attr || page != run_page)) {
            if (nruns < MAP_RUNS) {
                unsigned e = off + 8 + nruns * 16;
                put32(e, (u32)run_start);
                put32(e + 4, (u32)addr);
                put32(e + 8, (u32)run_attr);
                put32(e + 12, (u32)run_page);
            }
            nruns++;
            in_run = 0;
        }
        if (!in_run && rc == 0) {
            in_run = 1;
            run_start = addr;
            run_attr = attr;
            run_page = page;
        }
    }
    put64(off, nruns);
}

static void modules(void)
{
    sysPrxModuleList list;
    unsigned i, n;
    s32 rc;

    for (i = 0; i < 64; i++)
        idbuf[i] = lvbuf[i] = 0;
    list.size = sizeof(list);
    list.max = 64;
    list.count = 0;
    list.idlist = (u32)(unsigned long)idbuf;
    list.levellist = (u32)(unsigned long)lvbuf;
    rc = sysPrxGetModuleList(0, &list);
    put64(OFF_MODLIST, (u64)(s64)rc);
    put64(OFF_MODLIST + 8, list.count);
    n = list.count < MOD_MAX ? list.count : MOD_MAX;
    if (rc != 0)
        n = 0;
    for (i = 0; i < n; i++) {
        sysPrxModuleInfo info;
        unsigned e = OFF_MODINFO + i * MOD_REC, j;
        for (j = 0; j < sizeof(info); j++)
            ((unsigned char *)&info)[j] = 0;
        for (j = 0; j < SEG_MAX; j++) {
            segbuf[j].base = 0;
            segbuf[j].memsize = 0;
        }
        info.size = sizeof(info);
        info.filename = 0;
        info.filename_size = 0;
        info.segments = (u32)(unsigned long)segbuf;
        info.segments_num = SEG_MAX;
        rc = sysPrxGetModuleInfo(idbuf[i], 0, &info);
        put32(e, idbuf[i]);
        put32(e + 4, (u32)rc);
        put32(e + 8, info.all_segments_num);
        put32(e + 12, info.segments_num);
        for (j = 0; j < 30; j++)
            out[e + 16 + j] = info.name[j];
        for (j = 0; j < SEG_MAX; j++) {
            put64(e + 48 + j * 16, segbuf[j].base);
            put64(e + 56 + j * 16, segbuf[j].memsize);
        }
    }
}

int main(void)
{
    u64 r13, rc;
    unsigned i;

    __asm__ volatile ("mr %0, 13" : "=r"(r13));
    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    put64(OFF_HDR, r13);
#if IMAGE_PAD
    image_pad[0] = 1;
    image_pad[IMAGE_PAD - 1] = 1;
    put64(OFF_HDR + 8, EA(&image_pad[IMAGE_PAD]));
#endif
    put64(OFF_HDR + 16, IMAGE_PAD);
    put64(OFF_HDR + 24, EA(out));
    put64(OFF_HDR + 32, EA(main));

    sweep(OFF_MAP0);

    put64(OFF_CALLS + CALL_LOAD * 8, (u64)(s64)sysModuleLoad(SYSMODULE_FS));
    modules();
    sweep(OFF_MAP1);

    addr_word = 0;
    rc = sc(SYS_MEMORY_ALLOCATE, 0x10000, PAGE_64K, EA(&addr_word));
    put64(OFF_CALLS + CALL_ALLOC64K * 8, rc);
    put64(OFF_CALLS + CALL_ADDR64K * 8, rc == 0 ? addr_word : 0);
    {
        u64 a64 = rc == 0 ? addr_word : 0;
        u64 a1m;
        addr_word = 0;
        rc = sc(SYS_MEMORY_ALLOCATE, 0x100000, PAGE_1M, EA(&addr_word));
        a1m = rc == 0 ? addr_word : 0;
        put64(OFF_CALLS + CALL_ALLOC1M * 8, rc);
        put64(OFF_CALLS + CALL_ADDR1M * 8, a1m);
        sweep(OFF_MAP2);
        if (a64)
            put64(OFF_CALLS + CALL_FREE64K * 8, sc(SYS_MEMORY_FREE, a64, 0, 0));
        if (a1m)
            put64(OFF_CALLS + CALL_FREE1M * 8, sc(SYS_MEMORY_FREE, a1m, 0, 0));
    }

    put64(OFF_CALLS + CALL_UNLOAD * 8, (u64)(s64)sysModuleUnload(SYSMODULE_FS));
    write_frame();
    return 0;
}
