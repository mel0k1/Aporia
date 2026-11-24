#!/usr/bin/env bash


set -uo pipefail

fail() { printf 'parse_pr_command.sh: %s\n' "$1" >&2; exit 2; }

body="${1-}"
[ -n "$body" ] || body="$(cat)"

line="$(printf '%s\n' "$body" | grep -m1 -E '^[[:space:]]*/aster-code-review([[:space:]]|$)' || true)"
[ -n "$line" ] || fail "no '/aster-code-review' command found"

rest="$(printf '%s' "$line" | sed -E 's#^[[:space:]]*/aster-code-review[[:space:]]*##; s#[[:space:]]*$##')"
sub="${rest%%[[:space:]]*}"
args="$(printf '%s' "${rest#"$sub"}" | sed -E 's#^[[:space:]]*##')"
[ -n "$sub" ] || sub="diff"

case "$sub" in
    diff)
        [ -z "$args" ] || fail "'diff' takes no arguments"
        printf 'kind=review\nmode=diff\n' ;;
    files)
        [ -n "$args" ] || fail "'files' needs at least one path"
        for p in $args; do
            case "$p" in
                -*|/*|*..*) fail "invalid path: $p" ;;
            esac
            printf '%s' "$p" | grep -qE '^[A-Za-z0-9._/-]+$' || fail "invalid path: $p"
        done
        printf 'kind=review\nmode=files\npaths=%s\n' "$args" ;;
    smoke|benchmark)
        problems=""
        if [ -n "$args" ]; then
            case "$args" in
                --problems=*) problems="${args#--problems=}"; problems="${problems%\"}"; problems="${problems#\"}" ;;
                *) fail "'$sub' accepts only --problems=\"…\"" ;;
            esac
            printf '%s' "$problems" | grep -qE '^[0-9]{1,4}( [0-9]{1,4})*$' \
                || fail "--problems must be space-separated problem numbers, e.g. \"0002 0006\""
        fi
        printf 'kind=test\ntarget=%s\nproblems=%s\n' "$sub" "$problems" ;;
    *)
        fail "unknown subcommand '$sub' (use: diff | files | smoke | benchmark)" ;;
esac
