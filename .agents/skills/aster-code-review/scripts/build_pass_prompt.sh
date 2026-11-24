#!/usr/bin/env bash


set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SKILLDIR="$(cd "$HERE/.." && pwd)"
REPO="$(cd "$SKILLDIR/../../.." && pwd)"

if [[ -n "${ACR_GUIDELINE_ROOT:-}" ]]; then
    GROOT="$ACR_GUIDELINE_ROOT"
elif [[ -f "$SKILLDIR/guideline-root" ]]; then
    GROOT="$SKILLDIR/guideline-root"
else
    GROOT="$REPO"
fi

input="${1:-}"; shift || true
[[ -n "$input" && -f "$input" ]] || { echo "build_pass_prompt.sh: a readable <input-file> is required" >&2; exit 2; }
[[ $# -ge 1 ]] || { echo "build_pass_prompt.sh: at least one <persona> is required" >&2; exit 2; }

linked_guideline_pages() {
    python3 - "$1" "$GROOT/$1" <<'PY'
import posixpath
import re
import sys

root_page = sys.argv[1]
root_file = sys.argv[2]
base_dir = posixpath.dirname(root_page)
seen = set()

with open(root_file, encoding="utf-8") as file:
    page_text = file.read()

for link in re.findall(r"\[[^\]]+\]\(([^)]+)\)", page_text):
    link = link.split("#", 1)[0]
    if not link or "://" in link or link.startswith("#"):
        continue
    if link.endswith("/"):
        link += "README.md"
    path = posixpath.normpath(posixpath.join(base_dir, link))
    if not path.startswith("book/src/to-contribute/coding-guidelines/"):
        continue
    if path == root_page or path in seen:
        continue
    seen.add(path)
    print(path)
PY
}

inline_guidelines() {
    local gp="$1" gfile="$GROOT/$gp" sub
    [[ -f "$gfile" ]] || return 0

    printf 'Rely on the inlined guideline material below as the authoritative guideline context for this pass. Do not re-read `book/src/to-contribute/coding-guidelines/` from the worktree under review for guideline content.\n\n'
    printf -- '--- guideline page (%s) ---\n\n' "$gp"
    cat "$gfile"; printf '\n'

    while IFS= read -r sub; do
        [[ -f "$GROOT/$sub" ]] || continue
        printf '\n--- guideline subpage (%s) ---\n\n' "$sub"
        cat "$GROOT/$sub"; printf '\n'
    done < <(linked_guideline_pages "$gp")
}

cat "$SKILLDIR/scripts/pass_contract.md"
printf '\n'

for persona in "$@"; do
    pf="$SKILLDIR/personas/$persona.md"
    [[ -f "$pf" ]] || { echo "build_pass_prompt.sh: no such persona: $persona" >&2; exit 2; }
    printf '===== PERSONA: %s =====\n\n' "$persona"
    cat "$pf"; printf '\n'
    gp="$(grep -oE 'book/[A-Za-z0-9/_-]+README\.md' "$pf" | head -1 || true)"
    [[ -n "$gp" ]] && inline_guidelines "$gp"
done

printf '===== REVIEW INPUT =====\n\n'
cat "$input"
