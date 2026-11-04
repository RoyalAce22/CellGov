/* PPU probe: the CTRL value (mfspr 136) each of three PPU threads reads
 * while all three run at once on the PPE's two hardware threads. CTRL
 * bits 0:1 (CT) name the hardware thread that reads the register, so a
 * thread that runs on both hardware threads reads two values.
 *
 * The main thread starts two reader threads at its own priority, then
 * reads CTRL itself for the same window. Each reader records up to four
 * distinct CTRL values and how many reads returned each one.
 *
 * Payload: big-endian u64 words. Reader k (0 = main thread, 1 and 2 =
 * the started threads) has twelve words from 12 * k:
 *   +0 create rc (0 for the main thread; all ones: not created)
 *   +1 join rc (0 for the main thread; all ones: not joined)
 *   +2 exit value (expected 0x4354 for a started thread, 0 for main)
 *   +3 reads in all
 *   +4..+7 the distinct values, in the order first read (0: none)
 *   +8..+11 how many reads returned each of them
 * Word 36: the window in time-base ticks. Word 37: 0x43545254 ("CTRT"):
 * the probe wrote every word.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define MAIN_PRIORITY 1001

SYS_PROCESS_PARAM(MAIN_PRIORITY, 0x10000)

#define SYS_PPU_THREAD_EXIT  41
#define SYS_PPU_THREAD_JOIN  44

#define READERS  3
#define DISTINCT 4
#define WORDS    38
#define PAYLOAD  (WORDS * 8)

#define TB_HZ     79800000ull
/* Each reader reads CTRL for 2 s from its own start. */
#define WINDOW_TB (TB_HZ * 2ull)

struct reader {
    u64 reads;
    u64 value[DISTINCT];
    u64 count[DISTINCT];
};

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile struct reader rd[READERS];

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static struct cg_thread_param params[READERS] __attribute__((aligned(8)));
static unsigned long long tids[READERS];

static unsigned long make_thread_param(struct cg_thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    unsigned long tls_reg;
    __asm__ volatile ("mr %0, 13" : "=r"(tls_reg));
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = (unsigned int)tls_reg;
    return (unsigned long)p;
}

static inline u64 timebase(void)
{
    u64 tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static inline u64 read_ctrl(void)
{
    u64 v;
    __asm__ volatile ("mfspr %0, 136" : "=r"(v));
    return v;
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

/* Read CTRL for WINDOW_TB and tally the distinct values into rd[k]. */
static void read_window(unsigned k)
{
    u64 start = timebase();
    while (timebase() - start < WINDOW_TB) {
        u64 v = read_ctrl();
        unsigned i;
        rd[k].reads = rd[k].reads + 1;
        for (i = 0; i < DISTINCT; i++) {
            if (rd[k].count[i] == 0) {
                rd[k].value[i] = v;
                rd[k].count[i] = 1;
                break;
            }
            if (rd[k].value[i] == v) {
                rd[k].count[i] = rd[k].count[i] + 1;
                break;
            }
        }
    }
}

static void reader_entry(void *arg)
{
    read_window((unsigned)(unsigned long)arg);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x4354);
    for (;;) { }
}

static void put_reader(unsigned k)
{
    unsigned i;
    put(12 * k + 3, rd[k].reads);
    for (i = 0; i < DISTINCT; i++) {
        put(12 * k + 4 + i, rd[k].value[i]);
        put(12 * k + 8 + i, rd[k].count[i]);
    }
}

int main(void)
{
    unsigned i;
    s32 create_rc[READERS];
    u64 t0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 1; i < READERS; i++) {
        put(12 * i, ~0ull);
        put(12 * i + 1, ~0ull);
    }

    t0 = timebase();
    for (i = 1; i < READERS; i++) {
        create_rc[i] = cgov_ppu_thread_create(
            &tids[i], make_thread_param(&params[i], (const void *)&reader_entry),
            i, 0, MAIN_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
        put(12 * i, (u64)(s64)create_rc[i]);
    }
    read_window(0);
    put(36, timebase() - t0);

    for (i = 1; i < READERS; i++) {
        unsigned long long retval = 0;
        if (create_rc[i] != 0)
            continue;
        put(12 * i + 1, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tids[i],
                                               (unsigned long)&retval));
        put(12 * i + 2, retval);
    }
    for (i = 0; i < READERS; i++)
        put_reader(i);
    put(37, 0x43545254ull);

    write_frame();
    return 0;
}
