/* SPU program for spu_group_join_null.
 *
 * Spin for SPIN_ITERATIONS, so the PPU's join is issued while the
 * group runs, then end the group with sys_spu_thread_group_exit.
 */

#include <spu_intrinsics.h>
#include <sys/spu_thread.h>

#define SPIN_ITERATIONS 50000000u
#define EXIT_STATUS     7

int main(void)
{
    volatile unsigned int n;

    for (n = 0; n < SPIN_ITERATIONS; n++)
        ;
    spu_thread_group_exit(EXIT_STATUS);
    return 0;
}
