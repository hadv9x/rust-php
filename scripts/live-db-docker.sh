#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/docker/database/compose.yaml"
ACTION="${1:-test}"
shift || true

compose() {
    docker compose -f "$COMPOSE_FILE" --profile sqlserver "$@"
}

compose_oracle() {
    docker compose -f "$COMPOSE_FILE" --profile sqlserver --profile oracle "$@"
}

compose_test() {
    docker compose -f "$COMPOSE_FILE" --profile sqlserver --profile live-tests "$@"
}

compose_test_oracle() {
    docker compose -f "$COMPOSE_FILE" --profile sqlserver --profile oracle --profile live-tests "$@"
}

compose_all() {
    docker compose -f "$COMPOSE_FILE" --profile sqlserver --profile oracle --profile live-tests "$@"
}

case "$ACTION" in
    up)
        compose up -d --wait --wait-timeout 600 "$@"
        ;;
    down)
        compose_all down "$@"
        ;;
    ps)
        compose_all ps "$@"
        ;;
    logs)
        compose_all logs -f "$@"
        ;;
    test)
        compose up -d --wait --wait-timeout 600
        compose_test run --build --rm --no-deps live-tests
        ;;
    test-oracle)
        compose_oracle up -d --wait --wait-timeout 600
        export PHP_RUST_TEST_ORACLE_DSN="${PHP_RUST_TEST_ORACLE_DSN:-oci:dbq=//127.0.0.1:${PHP_RUST_TEST_ORACLE_PORT:-11521}/FREEPDB1}"
        export PHP_RUST_TEST_ORACLE_USERNAME="${PHP_RUST_TEST_ORACLE_USERNAME:-system}"
        export PHP_RUST_TEST_ORACLE_PASSWORD="${PHP_RUST_TEST_ORACLE_PASSWORD:-rustphp_dev_password}"
        compose_test_oracle run --build --rm --no-deps live-tests
        ;;
    *)
        echo "usage: $0 {up|test|test-oracle|down|ps|logs} [docker compose arguments...]" >&2
        exit 2
        ;;
esac
