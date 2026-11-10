#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$repo_root/target/release/php-rust"}
if [[ ! -x "$binary" ]]; then
    echo "php-rust executable not found or not executable: $binary" >&2
    exit 2
fi

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT INT TERM

count=0
for example in "$repo_root"/examples/*.php; do
    name=${example#"$repo_root"/}
    if "$binary" "$example" >"$scratch/stdout" 2>"$scratch/stderr"; then
        count=$((count + 1))
    else
        status=$?
        echo "FAIL $name (exit $status)" >&2
        cat "$scratch/stdout" >&2
        cat "$scratch/stderr" >&2
        exit 1
    fi
done

echo "PASS: all $count PHP examples"
