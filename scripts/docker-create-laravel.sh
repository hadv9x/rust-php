#!/bin/sh
set -eu

target=${1:-laravel-smoke-app}
parent=$(dirname -- "$target")
name=$(basename -- "$target")
mkdir -p "$parent"
parent=$(CDPATH= cd -- "$parent" && pwd)
target="$parent/$name"
[ ! -e "$target" ] || { echo "target already exists: $target" >&2; exit 1; }

docker run --rm \
    --user "$(id -u):$(id -g)" \
    --volume "$parent:/workspace" \
    --workdir /workspace \
    composer:2 create-project laravel/laravel "$name" --prefer-dist

printf 'Created Laravel application at %s\n' "$target"
printf 'After building php-rust:%s, smoke-test it with:\n' "$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$(dirname -- "$0")/../Cargo.toml" | head -n 1)"
printf '  ./scripts/docker-laravel-smoke.sh %s\n' "$target"
