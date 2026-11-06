#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf 'Usage: %s "commit message"\n' "$0" >&2
    exit 2
}

[[ $# -eq 1 ]] || usage
commit_message=$1
[[ -n $commit_message ]] || usage

git rev-parse --is-inside-work-tree >/dev/null
branch=$(git branch --show-current)
if [[ -z $branch ]]; then
    printf 'Error: cannot push from a detached HEAD.\n' >&2
    exit 1
fi
if ! git remote get-url origin >/dev/null 2>&1; then
    printf 'Error: remote "origin" is not configured.\n' >&2
    exit 1
fi
if [[ -z $(git status --porcelain) ]]; then
    printf 'Error: there are no changes to commit.\n' >&2
    exit 1
fi

git add -A
git commit -m "$commit_message"
git push -u origin "$branch"
