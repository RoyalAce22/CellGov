/* PPU probe: the fixed-point arithmetic instructions at their operand boundaries.
 *
 * cases.tsv lists the cases: the XO-form add, subtract, multiply and divide instructions in all
 * four Rc and OE forms, the high multiplies, and the D-form add,
 * subtract and multiply immediates, each over 0, 1, -1, the 32- and
 * 64-bit signed extremes and a value with a nonzero high word, with
 * XER SO, OV and CA clear and set on alternate cases.
 * cases.h is the table rendered; tests/micro/common/cgov_cases.h is the
 * driver and gives the frame layout.
 */

#include <sys/process.h>

#include "cases.h"
#include "cgov_cases.h"

SYS_PROCESS_PARAM(1001, 0x10000)

int main(void)
{
    return cgov_cases_main();
}
