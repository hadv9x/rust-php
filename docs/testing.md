# Build, install, and test

This guide checks the project from source and then verifies the installed CLI and HTTP server. The HTTP smoke test requires `curl`; source builds require Rust/Cargo and access to crates.io the first time Tokio and its dependencies are downloaded.

## 1. Fetch dependencies and run the workspace tests

Run these commands from the repository root:

```sh
cargo --version
rustc --version
cargo fetch --locked
cargo test --workspace --locked
cargo build -p php-cli --release --locked
./scripts/test-examples.sh target/release/php-rust
```

`cargo test` runs the parser/runtime/compiler unit and doc tests, plus the HTTP server tests. A successful run ends with all test summaries reporting `0 failed`. The release build produces both `target/release/php-rust` and `target/release/php-rust-server`.

The example runner executes each top-level `examples/*.php` file in a fresh process and fails on the first non-zero exit. It verifies successful execution, not exact output. It is included in CI after the release CLI build; live database and application integration tests need separate setup.

All test source lives in the repository-root `tests/` directory, grouped first by crate and then by source area: for example `tests/crates/php-compiler/compiler/`, `tests/crates/php-runtime/runtime/`, `tests/crates/php-db/db/`, `tests/crates/php-cli/cli/`, and `tests/crates/php-syntax/`. Internal tests that need private implementation details are included from their owning module with a `#[cfg(test)]` path, so test source stays separate while retaining access to that module's private API. Cargo integration targets such as compiler behavior tests and live database tests are explicitly wired to files under this root directory.

If dependencies have already been fetched and the machine is offline, these checks can use Cargo's cache:

```sh
cargo test --workspace --locked --offline
cargo build -p php-cli --release --locked --offline
```

## 2. Check the CLI binaries

```sh
./target/release/php-rust --version
./target/release/php-rust examples/hello.php
./target/release/php-rust-server --help
```

The first command prints the version, the example prints its PHP output, and the server help prints the `--root` and `--bind` options.

## 3. Install from this checkout

Install both executables into Cargo's configured bin directory:

```sh
cargo install --path crates/php-cli --bins --locked --force
```

If dependencies are already cached and the install should work offline:

```sh
cargo install --path crates/php-cli --bins --locked --force --offline
```

Confirm that both installed commands can be found and launched:

```sh
command -v php-rust
command -v php-rust-server
php-rust --version
php-rust-server --help
```

If the commands are not found, add Cargo's bin directory to `PATH` (usually `~/.cargo/bin`) and open a new shell. For an installation from the prebuilt Linux archive, run `./install.sh` after extracting it, then use `~/.local/bin/php-rust` and `~/.local/bin/php-rust-server` or add `~/.local/bin` to `PATH`. The prebuilt archive is created by the maintainer with `./scripts/package-linux.sh`; it is not built by the recipient's install command. To verify the Linux x86_64 archive and installer without changing your normal install, run from the repository root:

```sh
./scripts/package-linux.sh
VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
ARCHIVE="dist/php-rust-$VERSION-linux-x86_64.tar.gz"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT INT TERM
tar -xzf "$ARCHIVE" -C "$TEST_ROOT"
PACKAGE_DIR="$TEST_ROOT/php-rust-$VERSION-linux-x86_64"
"$PACKAGE_DIR/install.sh" --prefix "$TEST_ROOT/installed"
"$TEST_ROOT/installed/bin/php-rust" --version
"$TEST_ROOT/installed/bin/php-rust-server" --help
```

## 4. Exercise the installed web server over HTTP

The repository smoke-test script starts the selected binary on loopback, uses a temporary document root, and cleans up the server and test files when it exits. It checks query/form globals, front-controller routing, status and headers, static files, `HEAD`, `php://input`, multipart uploads and `move_uploaded_file`, session persistence across two requests, and 64 requests with up to 16 concurrent clients. The script is `scripts/smoke-test-server.sh`.

Build and run it against the release binary:

```sh
cargo build -p php-cli --release --locked
PHP_RUST_SERVER="$PWD/target/release/php-rust-server" TEST_PORT=3000 ./scripts/smoke-test-server.sh
```

Or run it against the executable installed by `cargo install`:

```sh
TEST_PORT=3000 ./scripts/smoke-test-server.sh
```

Every check should print `PASS`. Set `TEST_PORT` if 3000 is occupied. This concurrency check is a functional smoke test, not a throughput benchmark or a guarantee of a particular 10k-request capacity. The server limits active connections and closes each connection when the client requests it or after 100 keep-alive requests.

For a basic throughput measurement against an already running server, first make sure the document root has a PHP endpoint that returns HTTP 200. The load script reports status-code counts, elapsed time, approximate requests per second, and p50/p95/p99 client-observed latency:

```sh
./scripts/load-test-server.sh http://127.0.0.1:3000 10000 64
```

Arguments are base URL, request count, and parallel clients. Start below the server's `--max-in-flight` setting. To avoid overwriting the application document root, prepare a temporary one:

```sh
mkdir -p /tmp/php-rust-bench
printf '%s\n' '<?php echo "ok";' > /tmp/php-rust-bench/index.php
./target/release/php-rust-server --root /tmp/php-rust-bench --bind 127.0.0.1:3302
```

Keep that server running in one terminal and run `./scripts/load-test-server.sh http://127.0.0.1:3302 100000 64` in another. A `404` response is counted as failed, so check the printed `http_*` distribution before interpreting throughput. Each request uses a fresh curl process and connection; the result includes client and connection setup overhead and is a basic batch test, not production capacity. For sustained keep-alive load, an optional tool such as `wrk` can provide a separate measurement, for example `wrk -t4 -c64 -d60s --latency http://127.0.0.1:3302/`.

## 5. Build and run a versioned Docker image

Docker tags come from the workspace version in `Cargo.toml`; the Dockerfile checks that build argument against the source version and adds it as an OCI image label. For version `0.2.0`, the image is `php-rust:0.2.0`:

```sh
./scripts/docker-build.sh
docker image inspect php-rust:0.2.0 --format '{{ index .Config.Labels "org.opencontainers.image.version" }}'
./scripts/docker-run.sh /path/to/php-app 3000
```

The application directory must contain `public/index.php`. The run script mounts that directory into `/app` and binds the server to `127.0.0.1:3000`. The image includes runtime libraries for PostgreSQL, MySQL/MariaDB, MongoDB, and unixODBC; SQL Server and Oracle still require their vendor ODBC drivers.

To try the current Laravel release, create a fresh app using Composer's Docker image, then build and smoke-test it with the versioned runtime image:

```sh
./scripts/docker-build.sh
./scripts/docker-create-laravel.sh /tmp/laravel-smoke
./scripts/docker-laravel-smoke.sh /tmp/laravel-smoke 18080 /up
```

Or do the image build, exact version-label check, Laravel install, and HTTP smoke in one command. It removes the temporary app after success and preserves it after a failed smoke so the next iteration can reuse it:

```sh
./scripts/docker-laravel-check.sh
```

The smoke script starts `php-rust:<Cargo version>`, requests Laravel's `/up` health route, prints the response, and removes the container. A successful image build only verifies packaging. Laravel compatibility is established only if this HTTP smoke passes; broader verification should also exercise application routes, sessions, validation, database migrations, and the Laravel test suite. Laravel requires a broad PHP core/extension surface, so this project may fail during Composer autoload or application bootstrap; see [the documented runtime limits](web-server.md).

## Troubleshooting

- `cargo fetch` cannot resolve `index.crates.io`: restore DNS/network access, then retry `cargo fetch --locked`.
- `Address already in use`: choose another port, for example `TEST_PORT=3300`.
- `Connection refused`: check the server log printed when the smoke test fails, confirm the bind address/port, and check whether startup failed.
- `Permission denied` or `Operation not permitted` while binding: run the listener test on a machine/container that allows local TCP sockets. A passing unit test alone does not prove that the operating environment permits opening a port.
- `php-rust-server: cannot open document root`: the `--root` path must exist and be readable.
- Docker cannot access `/var/run/docker.sock`: verify the current user can access the Docker daemon, then retry the build and smoke commands.
