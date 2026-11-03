#!/usr/bin/env bash
set -euo pipefail

names=(MYSQL MARIADB POSTGRESQL MONGODB SQLSERVER ORACLE)
configured=0
for name in "${names[@]}"; do
    key="PHP_RUST_TEST_${name}_DSN"
    if [[ -n "${!key:-}" ]]; then
        configured=$((configured + 1))
        echo "LIVE  $name: configured"
    else
        echo "SKIP  $name: ${key} is unset"
    fi
done
if [[ "$configured" -eq 0 ]]; then
    echo "Set one or more PHP_RUST_TEST_{MYSQL,MARIADB,POSTGRESQL,MONGODB,SQLSERVER,ORACLE}_DSN variables and optional *_USERNAME, *_PASSWORD, *_ODBC_DRIVER variables." >&2
    exit 2
fi
cargo test -p php-db --test live_databases --offline -- --nocapture
