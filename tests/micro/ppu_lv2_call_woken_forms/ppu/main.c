/* PPU probe: the round trip of a handoff between two threads through
 * each kind of LV2 wait that blocks and is woken, each timed with mftb.
 *
 * Each loop makes 1 << 12 handoffs, five runs a loop. A handoff loop
 * runs main and one helper thread at main's priority, so each thread
 * keeps its own hardware thread and every wake comes from the other
 * thread's call. In each round main wakes the helper and then waits;
 * the helper wakes, wakes main, and waits for the next round. Each main
 * wait blocks until the helper's call wakes it.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, the baseline
 *    1  semaphore: main posts the go semaphore (sc 94) and waits on the
 *       back semaphore (sc 92); the helper waits on go and posts back.
 *    2  event flag: main sets bit 0 of the go flag (sc 87) and waits for
 *       bit 0 of the back flag, OR and clear (sc 85); the helper waits
 *       on go and sets back.
 *    3  event queue: main sends on the go port (sc 138) and receives
 *       from the back queue (sc 130); the helper receives from the go
 *       queue and sends on the back port.
 *    4  condition variable: both threads share one mutex. Main, holding
 *       it, sets the turn word to 1, signals the go condition (sc 108)
 *       and waits on the back condition (sc 107) while the turn word is
 *       1. The helper waits on go while the turn word is 0, sets it to
 *       0 and signals back. Each wait gives up the mutex and takes it
 *       back before it returns.
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   16 + L     loop L's first nonzero return code, or 0
 *   32  go semaphore create rc         33  back semaphore create rc
 *   34  semaphore helper create rc     35  semaphore helper join rc
 *   36  back semaphore destroy rc      37  go semaphore destroy rc
 *   38  go flag create rc              39  back flag create rc
 *   40  flag helper create rc          41  flag helper join rc
 *   42  back flag destroy rc           43  go flag destroy rc
 *   44  go queue create rc             45  back queue create rc
 *   46  go port create rc              47  back port create rc
 *   48  go port connect rc             49  back port connect rc
 *   50  queue helper create rc         51  queue helper join rc
 *   52  go port disconnect rc          53  back port disconnect rc
 *   54  go port destroy rc             55  back port destroy rc
 *   56  back queue destroy rc          57  go queue destroy rc
 *   58  mutex create rc                59  go condition create rc
 *   60  back condition create rc       61  main's mutex lock rc
 *   62  condition helper create rc     63  main's mutex unlock rc
 *   64  condition helper join rc       65  back condition destroy rc
 *   66  go condition destroy rc        67  mutex destroy rc
 *   72 + F  helper F's first nonzero wait rc, or 0 (F = loop - 1)
 *   76 + F  helper F's first nonzero wake rc, or 0
 *   80 + F  the handoffs helper F completed
 *   84  the condition helper's mutex lock rc
 *   85  the condition helper's mutex unlock rc
 *   88  the first setup or teardown step whose rc was nonzero: its word
 *       above, or 0 when every step returned 0
 *   89  runs a loop (5)
 *   90  0x5050_4C57 when the probe wrote every word
 *
 * A loop whose setup failed is skipped and its words stay 0.
 *
 * Payload: 96 u64 words, 768 bytes.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT              41
#define SYS_PPU_THREAD_JOIN              44
#define SYS_EVENT_FLAG_CREATE            82
#define SYS_EVENT_FLAG_DESTROY           83
#define SYS_EVENT_FLAG_WAIT              85
#define SYS_EVENT_FLAG_SET               87
#define SYS_SEMAPHORE_CREATE             90
#define SYS_SEMAPHORE_DESTROY            91
#define SYS_SEMAPHORE_WAIT               92
#define SYS_SEMAPHORE_POST               94
#define SYS_MUTEX_CREATE                 100
#define SYS_MUTEX_DESTROY                101
#define SYS_MUTEX_LOCK                   102
#define SYS_MUTEX_UNLOCK                 104
#define SYS_COND_CREATE                  105
#define SYS_COND_DESTROY                 106
#define SYS_COND_WAIT                    107
#define SYS_COND_SIGNAL                  108
#define SYS_EVENT_QUEUE_CREATE           128
#define SYS_EVENT_QUEUE_DESTROY          129
#define SYS_EVENT_QUEUE_RECEIVE          130
#define SYS_EVENT_PORT_CREATE            134
#define SYS_EVENT_PORT_DESTROY           135
#define SYS_EVENT_PORT_CONNECT_LOCAL     136
#define SYS_EVENT_PORT_DISCONNECT        137
#define SYS_EVENT_PORT_SEND              138
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

#define FLAG_WAIT_OR_CLEAR 0x12
#define PORT_LOCAL         1
#define QUEUE_SIZE         16

/* A handoff wait, in us: far over a handoff, so only a lost wake ends a
 * loop. */
#define HANDOFF_TIMEOUT_US 1000000u
#define HELPER_PRIO   1001
#define HELPER_STACK  0x4000

#define WORDS   96
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define LOOPS   5
#define FORMS   4
#define DONE    0x50504C57ull

enum { FORM_SEM, FORM_FLAG, FORM_QUEUE, FORM_COND };

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

/* Attribute blocks, each field its own 32-bit word. */
static const unsigned int sem_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0, 0, 0 };                    /* FIFO, private */
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };              /* FIFO, private, one waiter */
static const unsigned int mutex_attr[10] __attribute__((aligned(8))) =
    { 1, 0x20, 0x200, 0x2000, 0, 0, 0, 0, 0, 0 };      /* FIFO, one lock a holder */
static const unsigned int cond_attr[6] __attribute__((aligned(8))) =
    { 0x200, 0, 0, 0, 0, 0 };                           /* private */
static const unsigned int queue_attr[4] __attribute__((aligned(8))) =
    { 1, 1, 0, 0 };                                     /* FIFO, PPU queue */

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

/* Repack the toolchain's 24-byte ELFv1 descriptor into the kernel's
 * { u32 code; u32 toc } form, as lv2_destroy_parked does. */
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

static struct cg_thread_param helper_param[FORMS] __attribute__((aligned(8)));

static volatile unsigned int id_word __attribute__((aligned(8)));
/* Each form's go and back object: semaphores, flags, queues or
 * conditions. */
static volatile unsigned int go_id, back_id;
/* The event queue form's ports. */
static volatile unsigned int go_port, back_port;
/* The condition form's mutex and turn word. */
static volatile unsigned int mutex_id;
static volatile unsigned int turn;

static volatile u64 main_bits __attribute__((aligned(8)));
static volatile u64 helper_bits __attribute__((aligned(8)));
static volatile u64 main_event[4] __attribute__((aligned(8)));
static volatile u64 helper_event[4] __attribute__((aligned(8)));

static volatile s32 helper_wait_rc[FORMS], helper_wake_rc[FORMS];
static volatile u64 handoffs[FORMS];
static volatile s32 helper_lock_rc, helper_unlock_rc;

static u64 first_rc[LOOPS];
static u64 first_failed;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static inline u64 timebase(void)
{
    u64 tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

/* Record a setup or teardown step's rc at `slot`; keep the first that
 * failed. */
static u64 step(unsigned slot, u64 rc)
{
    put(slot, rc & 0xFFFFFFFFull);
    if ((rc & 0xFFFFFFFFull) && !first_failed)
        first_failed = slot;
    return rc & 0xFFFFFFFFull;
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

/* Wake the other thread through `form`'s object `id` (or port). */
static u64 wake(unsigned form, unsigned int id, unsigned int port)
{
    switch (form) {
    case FORM_SEM:
        return (u64)cgov_sc(SYS_SEMAPHORE_POST, id, 1, 0, 0, 0, 0, 0, 0);
    case FORM_FLAG:
        return (u64)cgov_sc(SYS_EVENT_FLAG_SET, id, 1, 0, 0, 0, 0, 0, 0);
    case FORM_QUEUE:
        return (u64)cgov_sc(SYS_EVENT_PORT_SEND, port, 0, 0, 0, 0, 0, 0, 0);
    default:
        return (u64)cgov_sc(SYS_COND_SIGNAL, id, 0, 0, 0, 0, 0, 0, 0);
    }
}

/* Wait on `form`'s object `id`. The condition form waits while the turn
 * word is `busy`. */
static u64 wait_on(unsigned form, unsigned int id, volatile u64 *bits,
                   volatile u64 *event, unsigned int busy)
{
    u64 rc = 0;
    switch (form) {
    case FORM_SEM:
        return (u64)cgov_sc(SYS_SEMAPHORE_WAIT, id, HANDOFF_TIMEOUT_US, 0, 0, 0, 0, 0, 0);
    case FORM_FLAG:
        return (u64)cgov_sc(SYS_EVENT_FLAG_WAIT, id, 1, FLAG_WAIT_OR_CLEAR, EA(bits),
                            HANDOFF_TIMEOUT_US, 0, 0, 0);
    case FORM_QUEUE:
        return (u64)cgov_sc(SYS_EVENT_QUEUE_RECEIVE, id, EA(event), HANDOFF_TIMEOUT_US,
                            0, 0, 0, 0, 0);
    default:
        while (turn == busy && rc == 0)
            rc = (u64)cgov_sc(SYS_COND_WAIT, id, HANDOFF_TIMEOUT_US, 0, 0, 0, 0, 0, 0)
                 & 0xFFFFFFFFull;
        return rc;
    }
}

/* CALLS handoffs through `form`; returns the time-base ticks they took.
 * A failed wake or wait ends the loop, so a lost handoff never waits out
 * more than one timeout. */
static u64 __attribute__((noinline)) handoff_calls(unsigned form)
{
    unsigned loop = form + 1;
    u64 i, rc = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        if (form == FORM_COND)
            turn = 1;
        rc = wake(form, go_id, go_port) & 0xFFFFFFFFull;
        if (rc)
            break;
        rc = wait_on(form, back_id, &main_bits, main_event, 1) & 0xFFFFFFFFull;
        if (rc)
            break;
    }
    t0 = timebase() - t0;
    if (rc && !first_rc[loop])
        first_rc[loop] = rc;
    return t0;
}

/* Keep loop `loop`'s fastest and slowest of REPS runs. */
static void keep(unsigned loop, u64 lo, u64 hi)
{
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

static void timed_baseline(void)
{
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 i, rc, bad = 0;
        u64 t = timebase();
        for (i = 0; i < CALLS; i++) {
            rc = (u64)cgov_sc(SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0, 0, 0, 0, 0);
            bad = bad ? bad : (rc & 0xFFFFFFFFull);
        }
        t = timebase() - t;
        if (bad && !first_rc[0])
            first_rc[0] = bad;
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    keep(0, lo, hi);
}

static void timed_handoffs(unsigned form)
{
    unsigned loop = form + 1;
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    for (r = 0; r < REPS && !first_rc[loop]; r++) {
        u64 t = handoff_calls(form);
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    keep(loop, lo, hi);
}

/* A handoff helper: answer each of main's wakes with a wake of its own,
 * REPS * CALLS times, and stop at the first failure. `arg` is the form. */
static void helper_entry(void *arg)
{
    unsigned form = (unsigned)(unsigned long)arg;
    u64 n;
    s32 rc;
    if (form == FORM_COND)
        helper_lock_rc = syscall2_s32(SYS_MUTEX_LOCK, mutex_id, 0);
    for (n = 0; n < (u64)REPS * CALLS; n++) {
        rc = (s32)wait_on(form, go_id, &helper_bits, helper_event, 0);
        if (rc) {
            helper_wait_rc[form] = rc;
            break;
        }
        if (form == FORM_COND)
            turn = 0;
        rc = (s32)wake(form, back_id, back_port);
        if (rc) {
            helper_wake_rc[form] = rc;
            break;
        }
    }
    handoffs[form] = n;
    if (form == FORM_COND)
        helper_unlock_rc = syscall1_s32(SYS_MUTEX_UNLOCK, mutex_id);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* Start the helper of `form`, run its loop and join it. `create` and
 * `join` are the words of the helper's create and join rc. */
static void run_form(unsigned form, unsigned create, unsigned join)
{
    unsigned long long tid = 0, retval = 0;
    s32 rc = cgov_ppu_thread_create(&tid,
        make_thread_param(&helper_param[form], (const void *)&helper_entry), form, 0,
        HELPER_PRIO, HELPER_STACK, THREAD_JOINABLE, 0);
    if (step(create, (u64)rc) != 0)
        return;
    timed_handoffs(form);
    if (form == FORM_COND)
        step(63, (u64)cgov_sc(SYS_MUTEX_UNLOCK, mutex_id, 0, 0, 0, 0, 0, 0, 0));
    step(join, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, tid, EA(&retval), 0, 0, 0, 0, 0, 0));
}

static void semaphore_form(void)
{
    u64 go_rc, back_rc;
    go_rc = step(32, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1,
                                  0, 0, 0, 0));
    go_id = id_word;
    back_rc = step(33, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1,
                                    0, 0, 0, 0));
    back_id = id_word;
    if (go_rc == 0 && back_rc == 0)
        run_form(FORM_SEM, 34, 35);
    if (back_rc == 0)
        step(36, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, back_id, 0, 0, 0, 0, 0, 0, 0));
    if (go_rc == 0)
        step(37, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, go_id, 0, 0, 0, 0, 0, 0, 0));
}

static void flag_form(void)
{
    u64 go_rc, back_rc;
    go_rc = step(38, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0,
                                  0, 0, 0, 0, 0));
    go_id = id_word;
    back_rc = step(39, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0,
                                    0, 0, 0, 0, 0));
    back_id = id_word;
    if (go_rc == 0 && back_rc == 0)
        run_form(FORM_FLAG, 40, 41);
    if (back_rc == 0)
        step(42, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, back_id, 0, 0, 0, 0, 0, 0, 0));
    if (go_rc == 0)
        step(43, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, go_id, 0, 0, 0, 0, 0, 0, 0));
}

static void queue_form(void)
{
    u64 gq_rc, bq_rc, gp_rc = 1, bp_rc = 1, gc_rc = 1, bc_rc = 1;
    gq_rc = step(44, (u64)cgov_sc(SYS_EVENT_QUEUE_CREATE, EA(&id_word), EA(queue_attr), 0,
                                  QUEUE_SIZE, 0, 0, 0, 0));
    go_id = id_word;
    bq_rc = step(45, (u64)cgov_sc(SYS_EVENT_QUEUE_CREATE, EA(&id_word), EA(queue_attr), 0,
                                  QUEUE_SIZE, 0, 0, 0, 0));
    back_id = id_word;
    if (gq_rc == 0) {
        gp_rc = step(46, (u64)cgov_sc(SYS_EVENT_PORT_CREATE, EA(&id_word), PORT_LOCAL, 0,
                                      0, 0, 0, 0, 0));
        go_port = id_word;
        if (gp_rc == 0)
            gc_rc = step(48, (u64)cgov_sc(SYS_EVENT_PORT_CONNECT_LOCAL, go_port, go_id,
                                          0, 0, 0, 0, 0, 0));
    }
    if (bq_rc == 0) {
        bp_rc = step(47, (u64)cgov_sc(SYS_EVENT_PORT_CREATE, EA(&id_word), PORT_LOCAL, 0,
                                      0, 0, 0, 0, 0));
        back_port = id_word;
        if (bp_rc == 0)
            bc_rc = step(49, (u64)cgov_sc(SYS_EVENT_PORT_CONNECT_LOCAL, back_port, back_id,
                                          0, 0, 0, 0, 0, 0));
    }
    if (gc_rc == 0 && bc_rc == 0)
        run_form(FORM_QUEUE, 50, 51);
    if (gc_rc == 0)
        step(52, (u64)cgov_sc(SYS_EVENT_PORT_DISCONNECT, go_port, 0, 0, 0, 0, 0, 0, 0));
    if (bc_rc == 0)
        step(53, (u64)cgov_sc(SYS_EVENT_PORT_DISCONNECT, back_port, 0, 0, 0, 0, 0, 0, 0));
    if (gp_rc == 0)
        step(54, (u64)cgov_sc(SYS_EVENT_PORT_DESTROY, go_port, 0, 0, 0, 0, 0, 0, 0));
    if (bp_rc == 0)
        step(55, (u64)cgov_sc(SYS_EVENT_PORT_DESTROY, back_port, 0, 0, 0, 0, 0, 0, 0));
    if (bq_rc == 0)
        step(56, (u64)cgov_sc(SYS_EVENT_QUEUE_DESTROY, back_id, 0, 0, 0, 0, 0, 0, 0));
    if (gq_rc == 0)
        step(57, (u64)cgov_sc(SYS_EVENT_QUEUE_DESTROY, go_id, 0, 0, 0, 0, 0, 0, 0));
}

static void cond_form(void)
{
    u64 m_rc, go_rc = 1, back_rc = 1;
    m_rc = step(58, (u64)cgov_sc(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr), 0, 0,
                                 0, 0, 0, 0));
    mutex_id = id_word;
    if (m_rc == 0) {
        go_rc = step(59, (u64)cgov_sc(SYS_COND_CREATE, EA(&id_word), mutex_id, EA(cond_attr),
                                      0, 0, 0, 0, 0));
        go_id = id_word;
        back_rc = step(60, (u64)cgov_sc(SYS_COND_CREATE, EA(&id_word), mutex_id,
                                        EA(cond_attr), 0, 0, 0, 0, 0));
        back_id = id_word;
    }
    turn = 0;
    if (go_rc == 0 && back_rc == 0
        && step(61, (u64)cgov_sc(SYS_MUTEX_LOCK, mutex_id, 0, 0, 0, 0, 0, 0, 0)) == 0)
        run_form(FORM_COND, 62, 64);
    if (back_rc == 0)
        step(65, (u64)cgov_sc(SYS_COND_DESTROY, back_id, 0, 0, 0, 0, 0, 0, 0));
    if (go_rc == 0)
        step(66, (u64)cgov_sc(SYS_COND_DESTROY, go_id, 0, 0, 0, 0, 0, 0, 0));
    if (m_rc == 0)
        step(67, (u64)cgov_sc(SYS_MUTEX_DESTROY, mutex_id, 0, 0, 0, 0, 0, 0, 0));
}

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    timed_baseline();
    semaphore_form();
    flag_form();
    queue_form();
    cond_form();

    for (i = 0; i < LOOPS; i++)
        put(16 + i, first_rc[i]);
    for (i = 0; i < FORMS; i++) {
        put(72 + i, (u64)(unsigned int)helper_wait_rc[i]);
        put(76 + i, (u64)(unsigned int)helper_wake_rc[i]);
        put(80 + i, handoffs[i]);
    }
    put(84, (u64)(unsigned int)helper_lock_rc);
    put(85, (u64)(unsigned int)helper_unlock_rc);
    put(88, first_failed);
    put(89, REPS);
    put(90, DONE);
    write_frame();
    return 0;
}
