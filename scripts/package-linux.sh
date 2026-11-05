#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
[ -n "$version" ] || { echo "could not read package version" >&2; exit 1; }

cd "$root"
cargo build -p php-cli --release --locked
stage="$root/target/package/php-rust-$version-linux-x86_64"
rm -rf "$stage"
mkdir -p "$stage"
install -m 0755 "$root/target/release/php-rust" "$stage/php-rust"
install -m 0755 "$root/target/release/php-rust-server" "$stage/php-rust-server"
install -m 0755 "$root/scripts/install.sh" "$stage/install.sh"
cat > "$stage/README.txt" <<'TXT'
php-rust Linux x86_64 runtime

Install for the current user:
  ./install.sh

Optional install location:
  ./install.sh --prefix /some/path

Run a supported PHP script:
  php-rust path/to/script.php

This package is a standalone command-line runtime and does not require PHP,
Rust, Cargo, or a web server to run. It supports only the PHP subset described
in the project README.

The php-rust-server executable starts the bundled HTTP server:
  php-rust-server --root public --bind 127.0.0.1:3000
TXT
mkdir -p "$root/dist"
archive="$root/dist/php-rust-$version-linux-x86_64.tar.gz"
tar -C "$(dirname "$stage")" -czf "$archive" "$(basename "$stage")"
printf 'Created %s\n' "$archive"
