

_fail=0
_note() { printf '       %s\n' "$1"; }

assert_eq() {
    [[ "$2" == "$3" ]] && return 0
    _fail=$((_fail + 1)); _note "$1: expected [$3], got [$2]"
}
assert_contains() {
    [[ "$2" == *"$3"* ]] && return 0
    _fail=$((_fail + 1)); _note "$1: output is missing [$3]"
}
assert_absent() {
    [[ "$2" != *"$3"* ]] && return 0
    _fail=$((_fail + 1)); _note "$1: output unexpectedly contains [$3]"
}
assert_before() {
    local pre_a="${2%%"$3"*}" pre_b="${2%%"$4"*}"
    [[ "$2" == *"$3"* && "$2" == *"$4"* && ${#pre_a} -lt ${#pre_b} ]] && return 0
    _fail=$((_fail + 1)); _note "$1: expected [$3] to appear before [$4]"
}

build_repo() {
    local r="$1"; mkdir -p "$r"
    git -C "$r" init -q
    git -C "$r" config user.email t@t; git -C "$r" config user.name t
    printf 'base\n' > "$r/a.txt"; git -C "$r" add a.txt
    git -C "$r" commit -q -m M0; git -C "$r" branch -M main
    git -C "$r" checkout -q -b feature
    printf 'base\nmore\n' > "$r/a.txt"; printf 'new\n' > "$r/b.txt"
    git -C "$r" add -A; git -C "$r" commit -q -m F1
    git -C "$r" checkout -q main
    printf 'only-on-main\n' > "$r/m.txt"; git -C "$r" add m.txt
    git -C "$r" commit -q -m M1
    git -C "$r" checkout -q feature
}

run_suite() {
    local suite cases c total=0 failed=0
    suite="$(basename "${BASH_SOURCE[1]}")"
    cases="$(declare -F | awk '{print $3}' | grep '^test_' | sort)"
    printf '# %s\n' "$suite"
    for c in $cases; do
        total=$((total + 1)); _fail=0
        TMP="$(mktemp -d)"
        declare -F setup    >/dev/null && setup
        "$c"
        declare -F teardown >/dev/null && teardown
        rm -rf "$TMP"
        if [[ $_fail -eq 0 ]]; then printf '  ok   %s\n' "${c#test_}"
        else                        printf '  FAIL %s\n' "${c#test_}"; failed=$((failed + 1)); fi
    done
    printf '  -- %s/%s cases passed in %s\n' "$((total - failed))" "$total" "$suite"
    [[ $failed -eq 0 ]]
}
