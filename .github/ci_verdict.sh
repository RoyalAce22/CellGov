#!/usr/bin/env bash
# Prints one Markdown row per job of a workflow run: its result, and for a
# job that is not green, the class of its failure. The class comes from the
# jobs API's step conclusions and timestamps, never from log text:
#
#   infrastructure  timed out, cancelled, or failed in a setup, cache,
#                   upload or post step
#   test            the `repository tests` step (or a step named
#                   `cargo test ...` / `cargo nextest ...`) failed
#   compile/lint    any other `repository ...` or `cargo ...` step, or
#                   the workflow syntax check, failed
#
# The step names are the interface: a new check step in a workflow is
# named `repository <group>` so this script classes its failure.
#   superseded      cancelled because a newer run of the same workflow on
#                   the same branch started before the job ended, on a
#                   ref where a newer run cancels an older one
#   running         not finished when this script read it
#
# A timed-out job and a superseded one both end `cancelled`, often in the
# same post step; the newer run's start time is what tells them apart.
# Where a newer run cancels nothing (the caller says so), every cancel is
# `infrastructure`.
#
# usage: ci_verdict.sh <owner/repo> <run id> [attempt] [job name to skip]
#                      [true|false: a newer run cancels this one (default true)]
# Needs `gh` authenticated with `actions: read` on the repository.

set -euo pipefail

repo=${1:?usage: ci_verdict.sh <owner/repo> <run id> [attempt] [job to skip]}
run=${2:?usage: ci_verdict.sh <owner/repo> <run id> [attempt] [job to skip]}
attempt=${3:-1}
skip=${4:-}
cancels=${5:-true}

run_fields=$(gh api "repos/$repo/actions/runs/$run" \
    --jq '"\(.workflow_id) \(.created_at) \(.head_branch)"')
read -r workflow created branch <<<"$run_fields"

# The earliest run of this workflow on this branch created after this one;
# empty when there is none. Timestamps are ISO 8601 in UTC, so string
# order is time order.
newer=
[ "$cancels" = true ] && newer=$(gh api "repos/$repo/actions/workflows/$workflow/runs?branch=$branch&per_page=100" \
    --jq "[.workflow_runs[] | select(.id != $run and .created_at > \"$created\") | .created_at] | min // \"\"")

printf '| job | result | class | first step not green | minutes |\n'
printf '|---|---|---|---|---|\n'
gh api "repos/$repo/actions/runs/$run/attempts/$attempt/jobs?per_page=100" --jq "
    .jobs[] | select(.name != \"$skip\") |
    (first(.steps[]? | select(.conclusion == \"failure\" or .conclusion == \"cancelled\" or .conclusion == \"timed_out\")) // {name: \"\"}) as \$bad |
    (if .conclusion == null then \"running\"
     elif .conclusion == \"success\" or .conclusion == \"skipped\" or .conclusion == \"neutral\" then \"-\"
     elif .conclusion == \"cancelled\" and \"$newer\" != \"\" and .completed_at >= \"$newer\" then \"superseded\"
     elif .conclusion == \"failure\" and (\$bad.name | test(\"^(repository tests|cargo test|cargo nextest)\")) then \"test\"
     elif .conclusion == \"failure\" and (\$bad.name | test(\"^(repository |cargo |Validate workflow)\")) then \"compile/lint\"
     else \"infrastructure\" end) as \$class |
    (if .started_at and .completed_at then (((.completed_at | fromdate) - (.started_at | fromdate)) / 60 * 10 | floor / 10 | tostring) else \"\" end) as \$minutes |
    \"| \(.name) | \(.conclusion // .status) | \(\$class) | \(\$bad.name) | \(\$minutes) |\"
"
