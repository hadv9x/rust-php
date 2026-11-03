#!/bin/sh
set -eu

prefix="${HOME}/.local"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] || { echo "usage: install.sh [--prefix DIR]" >&2; exit 2; }
            prefix=$2
            shift 2
            ;;
        --help|-h)
            echo "Usage: install.sh [--prefix DIR]"
            echo "Installs php-rust into DIR/bin (default: ~/.local/bin)."
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            echo "usage: install.sh [--prefix DIR]" >&2
            exit 2
            ;;
    esac
done

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
binary="$script_dir/php-rust"
[ -f "$binary" ] || { echo "php-rust executable not found beside installer" >&2; exit 1; }

target="$prefix/bin"
mkdir -p "$target"
install -m 0755 "$binary" "$target/php-rust"
if [ -f "$script_dir/php-rust-server" ]; then
    install -m 0755 "$script_dir/php-rust-server" "$target/php-rust-server"
fi
printf 'Installed php-rust to %s\n' "$target/php-rust"
case ":${PATH}:" in
    *":$target:"*) ;;
    *) printf 'Add this to your shell profile if needed: export PATH="%s:$PATH"\n' "$target" ;;
esac
printf 'Run a PHP script with: %s/php-rust path/to/script.php\n' "$target"
