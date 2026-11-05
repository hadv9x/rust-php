# Contributing

Thanks for helping improve `php-rust`. The project is an experimental PHP interpreter, so contributions should describe the PHP behavior they implement and the limits that remain.

## Before changing code

- Check the [compatibility backlog](docs/php-compatibility-backlog.md) and the relevant feature guide in [`docs/`](docs/README.md).
- Keep changes focused. Follow behavior through parsing, the AST, bytecode generation, and the runtime when those layers are involved.
- For behavior changes, add a focused regression test and a runnable PHP example when it helps explain the feature.
- Update the relevant documentation and compatibility notes. Avoid describing a partial implementation as full PHP compatibility.

## Build and verify

Use a stable Rust toolchain. From the repository root, run:

```sh
cargo check --workspace --locked
cargo test --workspace --locked
cargo build --release -p php-cli --locked
```

Run the affected examples with `cargo run -p php-cli -- examples/<name>.php`. For HTTP or database changes, use the relevant procedures in [testing](docs/testing.md); live database checks need explicitly configured test DSNs. The same workspace test, check, and CLI build commands run in GitHub Actions for pushes and pull requests.

## Pull requests

- Explain the user-visible behavior and any compatibility limitations.
- Include regression coverage and documentation for behavior changes.
- Report the commands you ran and whether any checks require external services.
- Keep credentials, local environment files, and generated build artifacts out of the change.

Project use and distribution are governed by the [MIT license](LICENSE).
