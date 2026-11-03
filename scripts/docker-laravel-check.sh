#!/bin/sh
set -eu

if [ "$#" -gt 1 ]; then
    echo "usage: scripts/docker-laravel-check.sh [port]" >&2
    exit 2
fi
port=${1:-18080}
case "$port" in ''|*[!0-9]*) echo "port must be numeric" >&2; exit 2 ;; esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/php-rust-laravel-check.XXXXXX")
keep_app=0
cleanup() {
    if [ "$keep_app" -eq 1 ]; then
        printf 'Laravel app kept for another run: %s/laravel\n' "$tmp" >&2
    else
        rm -rf "$tmp"
    fi
}
trap cleanup EXIT INT TERM

"$root/scripts/docker-build.sh"
image_version=$(docker image inspect "php-rust:$version" --format '{{ index .Config.Labels "org.opencontainers.image.version" }}')
[ "$image_version" = "$version" ] || {
    echo "image version label '$image_version' does not match source version '$version'" >&2
    exit 1
}
echo "Image version: $image_version"

"$root/scripts/docker-create-laravel.sh" "$tmp/laravel"
keep_app=1
"$root/scripts/docker-laravel-smoke.sh" "$tmp/laravel" "$port" /up
keep_app=0
