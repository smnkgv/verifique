#!/bin/sh
# Can a fresh agent connect a project with no help? It gets the output of `verifique init` and nothing
# else, on a copy with no real secrets. Run it on a copy that has no test stack of its own.
#
#   sh e2e/blind.sh <copy of a project> [seconds]
set -eu

PROJECT=$(cd "$1" && pwd)
LIMIT=${2:-3000}
LOG=$(mktemp -d "${TMPDIR:-/tmp}/verifique-blind-XXXXXX")
cd "$PROJECT"
[ ! -e verifique.yaml ] || { echo "this project is already connected" >&2; exit 1; }

{
    verifique init
    cat <<'RULES'


Work only from this guide and `verifique --help`. Do not read verifique's own source or documentation.
Nobody can answer questions during this run, so make your own decisions and write them down.

This copy has no real secrets. Make any `.env` from the project's example with local values only, and
never contact a remote service or account. Never stop processes by name or pattern. The queue is
already running, so do not start another.

Finish when `verifique doctor` passes, and a fast check and a `manual` agent check are defined. Then
submit one real `manual` check and wait for its verdict. Report what you did and every decision you made.
RULES
} >"$LOG/prompt.md"

timeout "$LIMIT" claude -p --output-format stream-json --verbose --permission-mode acceptEdits \
    --allowedTools Bash Read Write Edit Grep Glob <"$LOG/prompt.md" >"$LOG/run.jsonl"
verifique doctor | grep -q "doctor passed"
echo "blind setup passed: the agent's log is in $LOG"
