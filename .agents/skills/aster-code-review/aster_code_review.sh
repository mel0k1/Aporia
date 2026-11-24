#!/usr/bin/env bash


set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

[[ $# -ge 1 ]] || {
    echo "usage: ACR_AGENT_PROFILE=<name> aster_code_review.sh <diff|files> <args…> <output> [flags]" >&2
    exit 2; }

args=""
for tok in "$@"; do
    case "$tok" in
        *'"'*)          echo "aster_code_review.sh: a double quote in an argument is not supported: $tok" >&2; exit 2 ;;
        *[[:space:]]*)  tok="\"$tok\"" ;;
    esac
    args="${args:+$args }$tok"
done
prompt="Use the aster-code-review skill with these arguments: $args. Review this working tree."
exec "$HERE/scripts/run_agent.sh" "$prompt"
