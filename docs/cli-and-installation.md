# CLI and installation

The executable is provided by the `php-cli` crate; its binary name is `php-rust`. It accepts one PHP source file and optional script arguments per invocation.

## Run from a checkout

```sh
cargo run -p php-cli -- examples/hello.php
cargo run -p php-cli -- examples/loops.php
cargo run -p php-cli -- examples/classes.php
cargo run -p php-cli -- examples/cli-arguments.php --mode "two words"
```

Build/check the whole workspace with:

```sh
cargo check --workspace
```

## Install for the current user

With Rust and Cargo installed, run from the project root:

```sh
cargo install --path crates/php-cli --bins
```

Cargo installs `php-rust` and `php-rust-server` in `~/.cargo/bin`; ensure that directory is on `PATH`.

## Install a prebuilt Linux package

The release maintainer can create an x86_64 Linux archive with `scripts/package-linux.sh`. Extract the archive and run `./install.sh`; the installer places the executable in `~/.local/bin` by default. Rust/Cargo are needed to build from source, but not to run the prebuilt native executable.

## Arguments and exit status

- `php-rust <file.php> [script-arguments...]` reads, compiles, and executes one file. The script receives `$argc` and `$argv`; `$argv[0]` is the supplied script path and remaining entries are the arguments after it.
- `php-rust --help` prints usage; `--version` prints the executable version. These options are recognized when they are the first argument.
- Missing/unreadable input or invalid arguments exit with status 2; compile/runtime errors exit with status 1.

The `php-rust-server` HTTP binary is available; PHP-FPM/FastCGI and installers for other platforms/architectures are not currently provided. See the [web server guide](web-server.md) and [build/install test guide](testing.md) for an end-to-end check.
