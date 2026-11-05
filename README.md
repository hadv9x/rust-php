# php-rust

[![Rust CI](https://github.com/hadv9x/rust-php/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/hadv9x/rust-php/actions/workflows/ci.yml)

`php-rust` is an experimental PHP interpreter written in Rust. It parses PHP source, compiles the supported language subset to bytecode, and executes that bytecode in a virtual machine. The project provides a command-line runner and an HTTP development server.

This project is **not a drop-in replacement for PHP**. Language and standard-library coverage are actively being expanded, and compatibility with existing PHP applications depends on which features they use. See the [compatibility backlog](docs/php-compatibility-backlog.md) and [manual coverage audit](docs/php-manual-coverage.md) for the current scope and open work.

## Quick start

You need a Rust toolchain with Cargo. From the repository root:

```sh
cargo run -p php-cli -- examples/hello.php
```

To run a script with arguments:

```sh
cargo run -p php-cli -- examples/cli-arguments.php --mode "two words"
```

Build and run the CLI directly:

```sh
cargo build --release -p php-cli
./target/release/php-rust --help
./target/release/php-rust examples/hello.php
```

The CLI accepts `php-rust <file.php> [script-arguments...]`, `--help`, and `--version`. Script arguments are exposed as `$argc` and `$argv`.

## Install from source

Install both CLI and server binaries for the current user:

```sh
cargo install --path crates/php-cli --bins
```

Cargo installs `php-rust` and `php-rust-server` in `~/.cargo/bin`. Add that directory to `PATH` if needed. The installed programs are native Rust executables and do not require a separate PHP installation.

For a prebuilt Linux x86_64 package, see [CLI and installation](docs/cli-and-installation.md). The package maintainer workflow is documented there and in the packaging scripts.

## Run the HTTP server

The server serves files from a document root and sends missing routes to `index.php` when present. Start it from the repository root:

```sh
mkdir -p public
cp examples/hello.php public/index.php
cargo run -p php-cli --bin php-rust-server -- --root public --bind 127.0.0.1:3000
```

Then open <http://127.0.0.1:3000/>. The prototype initializes common request globals, handles PHP response headers and status codes, and supports static includes and Composer autoloading with documented limits. It is not PHP-FPM/FastCGI and does not provide direct HTTPS. See the [web server guide](docs/web-server.md) for configuration, request behavior, limits, and operational details.

## What is implemented

The runtime currently covers a growing subset of PHP, including:

- Variables, scalar values, expressions, arrays, common control flow, functions, closures, and generators.
- Namespaces, references, exceptions, and a substantial subset of classes, interfaces, traits, enums, and typed properties.
- Selected built-in functions for arrays, strings, JSON, math, dates, filesystem access, and CLI/runtime introspection.
- An HTTP server, a partial PDO-compatible SQL layer, and a MongoDB integration surface.

These areas are incomplete and may differ from PHP in edge cases, diagnostics, extensions, and function signatures. The detailed guides track behavior and known gaps: [syntax](docs/syntax-and-parser.md), [functions](docs/functions.md), [arrays](docs/array-compatibility.md), [object-oriented features](docs/oop-compatibility.md), [database support](docs/database.md), and [PHP manual coverage](docs/php-manual-coverage.md).

## Build and develop

Build or check the Rust workspace:

```sh
cargo build --workspace
cargo check --workspace
```

Run the workspace test suite:

```sh
cargo test --workspace
```

The workspace is split into these crates:

| Crate | Purpose |
| --- | --- |
| `php-syntax` | Lexer, parser, diagnostics, and PHP abstract syntax tree. |
| `php-compiler` | Compiles the supported syntax and declarations into runtime bytecode. |
| `php-runtime` | PHP values, built-ins, bytecode definitions, and virtual machine. |
| `php-db` | Database drivers, connection pools, and backend integrations. |
| `php-cli` | `php-rust` command-line runner and `php-rust-server` HTTP server. |

See [development](docs/development.md), [compiler architecture](docs/compiler-architecture.md), and [testing](docs/testing.md) for contributor workflows and verification commands. The [`examples/`](examples/) directory contains runnable PHP samples; not every PHP feature or external database example runs without additional setup.

## Documentation

Start with the [documentation index](docs/README.md). It links to feature guides, compatibility notes, server and database documentation, and implementation details.

## Contributing

Contributions are welcome. Read the [contribution guide](CONTRIBUTING.md) for the development workflow and checks run by CI. The project is distributed under the [MIT license](LICENSE).

See the [open-source maintenance checklist](docs/open-source-maintenance.md) for current CI coverage and repository settings maintainers still need to enable.

## Docker

Scripts are provided to build a versioned Docker image and run the HTTP server against an application directory:

```sh
./scripts/docker-build.sh
./scripts/docker-run.sh /path/to/app 3000
```

The Laravel smoke workflow is an integration check, not a claim of broad Laravel or PHP compatibility. See [testing](docs/testing.md) for details.

## Release

To publish a version, push a matching `vX.Y.Z` tag. GitHub Actions tests the tagged source and creates a GitHub Release with the Linux package, Docker image, source archive, and checksums. Follow the [release process](docs/release-process.md).
