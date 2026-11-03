#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
app_path=${1:-.}
port=${2:-3000}

case "$port" in
    ''|*[!0-9]*) echo "port must be numeric" >&2; exit 2 ;;
esac
[ -f "$app_path/public/index.php" ] || { echo "Laravel/PHP application entry point not found: $app_path/public/index.php" >&2; exit 1; }

app_path=$(CDPATH= cd -- "$app_path" && pwd)
exec docker run --rm \
    --user "$(id -u):$(id -g)" \
    --publish "127.0.0.1:$port:3000" \
    --volume "$app_path:/app" \
    "php-rust:$version" \
    --root /app/public --bind 0.0.0.0:3000
