#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 3 ]; then
    echo "usage: scripts/docker-laravel-smoke.sh LARAVEL_APP [port] [path]" >&2
    exit 2
fi

app_path=$1
port=${2:-18080}
path=${3:-/up}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
container="php-rust-laravel-smoke-$$"

[ -f "$app_path/artisan" ] && [ -f "$app_path/public/index.php" ] || {
    echo "not a Laravel application root (artisan and public/index.php are required)" >&2
    exit 1
}
command -v curl >/dev/null 2>&1 || { echo "curl is required for the HTTP smoke check" >&2; exit 1; }
case "$port" in ''|*[!0-9]*) echo "port must be numeric" >&2; exit 2 ;; esac
case "$path" in /*) ;; *) echo "path must start with /" >&2; exit 2 ;; esac

cleanup() { docker rm -f "$container" >/dev/null 2>&1 || true; }
trap cleanup EXIT INT TERM
docker run --detach --name "$container" \
    --user "$(id -u):$(id -g)" \
    --publish "127.0.0.1:$port:3000" \
    --volume "$(CDPATH= cd -- "$app_path" && pwd):/app" \
    "php-rust:$version" --root /app/public --bind 0.0.0.0:3000 >/dev/null

attempt=0
while [ "$attempt" -lt 30 ]; do
    status=$(curl --silent --show-error --output /tmp/php-rust-laravel-smoke.out --write-out '%{http_code}' "http://127.0.0.1:$port$path" 2>/dev/null || true)
    case "$status" in
        2??)
            printf 'Laravel HTTP smoke: PASS (php-rust:%s, GET %s, HTTP %s)\n' "$version" "$path" "$status"
            cat /tmp/php-rust-laravel-smoke.out
            exit 0
            ;;
        000) ;;
        *)
            printf 'Laravel HTTP smoke failed (GET %s returned HTTP %s)\n' "$path" "$status" >&2
            cat /tmp/php-rust-laravel-smoke.out >&2
            docker logs "$container" >&2 || true
            exit 1
            ;;
    esac
    if ! docker inspect -f '{{.State.Running}}' "$container" 2>/dev/null | grep -q true; then
        echo "Laravel container exited before becoming ready" >&2
        docker logs "$container" >&2 || true
        exit 1
    fi
    attempt=$((attempt + 1))
    sleep 1
done

echo "Laravel did not return a successful HTTP response at GET $path" >&2
docker logs "$container" >&2 || true
exit 1
