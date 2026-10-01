/* PPU program: spawn a child SELF via _sys_process_spawn (LV2 #21)
 * and observe its exit by polling sys_process_get_status (LV2 #4).
 *
 * The spawn argument block follows the marshalled layout vsh's
 * client library builds (0x608950): { u64 table_off = 16; u64;
 * ptr table [argv..., NULL, envp..., NULL]; packed strings } with
 * argv[0] naming the SELF path. This mirrors the minimal
 * audio-fallback spawn (vsh 0xcb484):
 *   sc 21(pid_out, path, argv=NULL, envp=NULL, prio=1000, flags=0)
 *
 * Output layout (fixed at RESULT_ADDR in the PARENT's address
 * space; also TTY-reported as CGOV magic + 16 bytes):
 *   status    (u32)  0 = pass; bit 0 = spawn failed,
 *                    bit 1 = poll loop exhausted without observing
 *                    the child's exit
 *   pid       (u32)  child pid the kernel wrote back
 *   spawn_rc  (u32)  raw return of sc 21
 *   polls     (u32)  get_status polls before the exit was observed
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PROCESS_GET_STATUS 4
#define SYS_PROCESS_SPAWN      21

#define RESULT_ADDR 0x100
#define MAX_POLLS   2000000u

/* The console build (package_ps3.sh) defines CGOV_PS3_USRDIR: webMAN
 * does not map /app_home to the EBOOT's directory there, so the child
 * is named by the directory the package deploys into. */
#ifdef CGOV_PS3_USRDIR
#define CHILD_PATH  CGOV_PS3_USRDIR "/child.self"
#else
#define CHILD_PATH  "/app_home/child.self"
#endif

struct TestResult {
    unsigned int status;
    unsigned int pid;
    unsigned int spawn_rc;
    unsigned int polls;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };
static const char child_path[] = CHILD_PATH;

/* Marshalled spawn block: header (2 x u64), pointer table
 * (argv[0]=path, NULL argv terminator, NULL envp terminator),
 * strings packed separately (child_path above). */
static unsigned long long spawn_block[5] __attribute__((aligned(16)));

static struct TestResult result;

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
    CGOV_OUT_FILE_WRITE(r, len);
}

/* The copy at RESULT_ADDR is for CellGov's observation of the parent's
 * space. The console build (CGOV_OUT_FILE defined) skips it: 0x100 is
 * not mapped in a console process, and the frame file carries the
 * result there. */
static void publish(const struct TestResult *r)
{
#ifndef CGOV_OUT_FILE
    volatile struct TestResult *fixed =
        (volatile struct TestResult *)RESULT_ADDR;
    fixed->status = r->status;
    fixed->pid = r->pid;
    fixed->spawn_rc = r->spawn_rc;
    fixed->polls = r->polls;
#endif
    write_tty_result(r);
}

int main(void)
{
    static unsigned int pid_out __attribute__((aligned(8)));
    unsigned int polls = 0;
    s32 rc;

    spawn_block[0] = 16;   /* table_off */
    spawn_block[1] = 0;
    spawn_block[2] = (unsigned long long)(unsigned long)child_path;
    spawn_block[3] = 0;    /* argv terminator */
    spawn_block[4] = 0;    /* envp terminator */

#ifdef CGOV_OUT_FILE
    /* The console build leaves a stage in the result file before the
     * spawn and after it returns, so a run that never finishes still
     * says how far it got: status 0x80 = the spawn was entered, 0x40 =
     * it returned and the poll began. The last write is the final
     * result. */
    result.status = 0x80;
    CGOV_OUT_FILE_WRITE(&result, sizeof(result));
#endif
    rc = syscall6_s32(
        SYS_PROCESS_SPAWN,
        (unsigned long)&pid_out,
        1000,
        0,
        (unsigned long)spawn_block,
        sizeof(spawn_block) + sizeof(child_path),
        0);
#ifdef CGOV_OUT_FILE
    result.status = 0;
#endif
    result.spawn_rc = (unsigned int)rc;
    if (rc != 0) {
        result.status |= 0x1;
        publish(&result);
        return (int)result.status;
    }
    result.pid = pid_out;
#ifdef CGOV_OUT_FILE
    result.status = 0x40;
    CGOV_OUT_FILE_WRITE(&result, sizeof(result));
    result.status = 0;
#endif

    /* CELL_OK while the child lives; CELL_ESRCH once it has
     * exited. Any nonzero return ends the poll. */
    while (polls < MAX_POLLS) {
        if (syscall2_s32(SYS_PROCESS_GET_STATUS, pid_out, 0) != 0)
            break;
        polls++;
    }
    result.polls = polls;
    if (polls >= MAX_POLLS)
        result.status |= 0x2;

    publish(&result);
    return (int)result.status;
}
