/* PPU probe: what sys_game_process_exitspawn2 hands the game process it
 * starts.
 *
 * The program runs twice. The first run, the parent, creates a 1 MiB
 * memory container (341), leaves a marker file, and exitspawns into its
 * own EBOOT.BIN, naming a priority of 1500 and the 128 KiB primary-stack
 * flag. Both differ from the program's own process parameters (1001,
 * 64 KiB), so the second run's readings show which side wins. The second
 * run, the child, finds the marker, removes it and writes the payload.
 *
 * Payload: big-endian u64 words, one per slot.
 *    0  0xC0FFEE (the child ran), or 0xBAD (the parent's exitspawn
 *       returned; slots 1-2 then hold the 341 rc and container id)
 *    1  sys_process_getpid              2  sys_process_getppid
 *    3  sys_ppu_thread_get_priority rc  4  the priority
 *    5  r1 at main
 *    6  352 rc    7  user memory total    8  user memory available
 *    9  zero
 *
 * The marker needs the console build's file output; the reference build
 * always takes the parent's path.
 *
 * Started by the console runner (webMAN's /play.ps3), the parent reaches
 * the exitspawn and the second run never starts: the title hangs until
 * webMAN's exit clears it. The probe waits for a launch path that
 * carries an exitspawn.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>
#include <lv2/process.h>
#include <lv2/thread.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#ifndef CGOV_PS3_USRDIR
#define CGOV_PS3_USRDIR "/app_home"
#endif

#define SYS_MEMORY_CONTAINER_CREATE     341
#define SYS_MEMORY_GET_USER_MEMORY_SIZE 352
#define CHILD_PRIO        1500
#define STACK_SIZE_128K   0x40ULL
#define MIB               0x100000ULL
#define SKIP              0xDEADD00DULL

#define WORDS   10
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile unsigned int info[2] __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };
static const char SELF_PATH[] = CGOV_PS3_USRDIR "/EBOOT.BIN";

#define EA(p) ((u64)(unsigned long)(const void *)(p))

#ifdef CGOV_OUT_FILE
#define SYS_FS_UNLINK 814
#define CELL_FS_O_RDONLY 0
static const char MARKER[] = "/dev_hdd0/tmp/cgov_lv2_exitspawn_parent";

/* Leave the marker the second run looks for. */
static void leave_marker(void)
{
    int fd = -1;
    if (cgov_out_sc6(CGOV_OUT_SYS_FS_OPEN, CGOV_OUT_EA(MARKER), CGOV_OUT_OPEN_FLAGS,
                     CGOV_OUT_EA(&fd), CGOV_OUT_OPEN_MODE, 0, 0) == 0)
        cgov_out_sc1(CGOV_OUT_SYS_FS_CLOSE, (unsigned long long)fd);
}

/* Whether the parent left the marker; a found marker is removed. */
static int take_marker(void)
{
    int fd = -1;
    if (cgov_out_sc6(CGOV_OUT_SYS_FS_OPEN, CGOV_OUT_EA(MARKER), CELL_FS_O_RDONLY,
                     CGOV_OUT_EA(&fd), 0, 0, 0) != 0)
        return 0;
    cgov_out_sc1(CGOV_OUT_SYS_FS_CLOSE, (unsigned long long)fd);
    cgov_out_sc1(SYS_FS_UNLINK, CGOV_OUT_EA(MARKER));
    return 1;
}
#else
static void leave_marker(void) {}
static int take_marker(void) { return 0; }
#endif

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

static void child(u64 sp)
{
    sys_ppu_thread_t self = 0;
    s32 prio = -1;

    put(0, 0xC0FFEE);
    put(1, (u64)(u32)sysProcessGetPid());
    put(2, (u64)(u32)sysProcessGetPpid());
    sysThreadGetId(&self);
    put(3, (u64)(s64)sysThreadGetPriority(self, &prio));
    put(4, (u64)(s64)prio);
    put(5, sp);
    info[0] = 0xFFFFFFFFu;
    info[1] = 0xFFFFFFFFu;
    put(6, (u64)cgov_sc(SYS_MEMORY_GET_USER_MEMORY_SIZE, EA(info), 0, 0, 0, 0, 0, 0, 0));
    put(7, info[0]);
    put(8, info[1]);
    write_frame();
}

int main(void)
{
    unsigned i;
    u64 sp, rc;
    const char *argv[] = { SELF_PATH, 0 };

    __asm__ volatile ("mr %0, 1" : "=r"(sp));
    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < WORDS - 1; i++)
        put(i, SKIP);

    if (take_marker()) {
        child(sp);
        return 0;
    }

    w32 = 0xFFFFFFFFu;
    rc = (u64)cgov_sc(SYS_MEMORY_CONTAINER_CREATE, EA(&w32), MIB, 0, 0, 0, 0, 0, 0);
    leave_marker();
    sysProcessExitSpawn2(SELF_PATH, argv, 0, 0, 0, CHILD_PRIO, STACK_SIZE_128K);

    put(0, 0xBAD);
    put(1, rc);
    put(2, w32);
    write_frame();
    return 0;
}
