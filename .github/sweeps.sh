#!/usr/bin/env bash
# The scheduled exhaustive sweeps: ignored tests that run an instruction
# or a rounding step over its whole input space. Each has a bounded,
# fixed-seed form in the continuous build; these are the full spaces.
#
# Each job runs its tests by exact name and fails unless every one ran:
# a filter that matches no test passes with "0 passed".

set -euo pipefail

out="${SWEEPS_OUT:-target/sweeps}"

# Runs the ignored test `$3` of package `$1` (target `$2`: `lib` or an
# integration test) in release, logs it under `$out`, and fails unless it
# ran and passed.
ignored() {
    local package=$1 target=$2 test=$3 log
    mkdir -p "$out"
    log="$out/${test//::/-}.log"
    if [ "$target" = lib ]; then
        cargo test --release -p "$package" --locked --lib -- --ignored --exact "$test" 2>&1 | tee "$log"
    else
        cargo test --release -p "$package" --locked --test "$target" -- --ignored --exact "$test" 2>&1 | tee "$log"
    fi
    case "$(< "$log")" in
        *"test result: ok. 1 passed"*) ;;
        *) echo "sweeps: $test did not run" >&2; return 1 ;;
    esac
}

# The release times beside each job are one run on a Windows workstation
# (2026-10-21). None comes near the step's 170-minute bound, so no job is
# sharded.

# 78 s: unpack then round_pack over every single-precision pattern.
round_trip() {
    ignored cellgov_float lib round_tests::every_single_precision_pattern_round_trips
}

# 241 s: the four integer and float conversions over every operand.
conversion() {
    ignored cellgov_spu lib exec::dispatch::conversion_tests::every_operand_at_scale_0_matches_the_oracle
}

# 37 s, 6 s and 9 s: the estimate sequences over every fraction.
estimates() {
    ignored cellgov_spu lib exec::dispatch::estimate_tests::both_sequences_meet_their_bounds_for_every_fraction
    ignored cellgov_spu lib exec::dispatch::estimate_tests::the_frsqest_zero_threshold_holds_for_every_fraction
    ignored cellgov_spu it spu_fi_datapath::both_sequences_match_the_console_digest_over_every_fraction
}

# 682 s: the four compares over every first operand.
compare() {
    ignored cellgov_spu lib exec::dispatch::float_compare_tests::every_first_operand_orders_as_its_value
}

# 129 s: fesd over every single.
fesd() {
    ignored cellgov_spu lib exec::dispatch::precision_conversion_tests::fesd_matches_the_oracle_for_every_single
}

# 969 s: fm over every first operand against the exact host product.
fm() {
    ignored cellgov_spu lib exec::dispatch::single_float_tests::fm_over_every_first_operand_matches_the_exact_host_product
}

case "${1:-}" in
    round-trip) round_trip ;;
    conversion) conversion ;;
    estimates) estimates ;;
    compare) compare ;;
    fesd) fesd ;;
    fm) fm ;;
    *)
        echo "usage: $0 {round-trip|conversion|estimates|compare|fesd|fm}" >&2
        exit 2
        ;;
esac
