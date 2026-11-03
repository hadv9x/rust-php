#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
[ -n "$version" ] || { echo "could not read workspace version from Cargo.toml" >&2; exit 1; }

if [ "$#" -gt 1 ]; then
    echo "usage: scripts/docker-build.sh [expected-version]" >&2
    exit 2
fi
if [ "$#" -eq 1 ] && [ "$1" != "$version" ]; then
    echo "requested version '$1' does not match Cargo workspace version '$version'" >&2
    exit 1
fi

docker build \
    --build-arg "PHP_RUST_VERSION=$version" \
    --label "org.opencontainers.image.version=$version" \
    --tag "php-rust:$version" \
    --file "$root/Dockerfile" \
    "$root"

printf 'Built php-rust:%s\n' "$version"
