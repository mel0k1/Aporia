#!/usr/bin/env bash


set -euo pipefail
wt="${1:?usage: overlay_skill.sh <worktree>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
SKILL="$(cd "$HERE/.." && pwd)"

rm -rf "$wt/.agents" "$wt/.claude"
mkdir -p "$wt/.agents/skills"
cp -r "$SKILL" "$wt/.agents/skills/aster-code-review"
rm -rf "$wt/.agents/skills/aster-code-review/benchmark"
rm -rf "$wt/.agents/skills/aster-code-review/guideline-root"
mkdir -p "$wt/.agents/skills/aster-code-review/guideline-root/book/src/to-contribute"
cp -r "$HERE/../../../../book/src/to-contribute/coding-guidelines" \
    "$wt/.agents/skills/aster-code-review/guideline-root/book/src/to-contribute/"

mkdir -p "$wt/.claude/skills"
ln -sfn "../../.agents/skills/aster-code-review" "$wt/.claude/skills/aster-code-review"
