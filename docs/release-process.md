# Release process

Releases are created from version tags. The tag must match the workspace version in `Cargo.toml`; for example, version `0.1.0` uses tag `v0.1.0`.

## Publish a release

1. Make sure the desired source and release workflow are committed and pushed to `master`.
2. Confirm the workspace version in `Cargo.toml` and that the matching tag does not already exist:

   ```sh
   git tag --list 'v*'
   ```

3. Create and push an annotated tag:

   ```sh
   git tag -a v0.1.0 -m "php-rust 0.1.0"
   git push origin v0.1.0
   ```

4. Open the repository's **Actions** tab and wait for the **Release** workflow. It runs workspace tests, verifies the tag/version match, builds the Linux x86_64 package and Docker image, creates a source archive and SHA-256 checksums, then publishes a GitHub Release with those assets.

For later releases, update the workspace version first, commit and push the change, then tag that commit with the matching `vX.Y.Z` value. The workflow rejects mismatched tag and manifest versions. Do not reuse a published version or move an existing release tag.

## Release assets

The workflow attaches:

- `php-rust-X.Y.Z-linux-x86_64.tar.gz` — standalone CLI and HTTP server executables with the user installer.
- `php-rust-X.Y.Z-source.tar.gz` — source snapshot for the tagged commit.
- `php-rust-X.Y.Z-docker.tar.gz` — compressed Docker image archive.
- `SHA256SUMS` — checksums for the three archives.

Verify downloaded archives with:

```sh
sha256sum --check SHA256SUMS
```

Install the Linux archive by extracting it and running `./install.sh`. Load the container archive with `docker load --input php-rust-X.Y.Z-docker.tar.gz`, then run it with the documented [Docker instructions](../README.md#docker). The Linux binary archive currently targets x86_64; releases for macOS, Windows, and other architectures are not produced.

## Local build without publishing

To build and package the current checkout without creating a GitHub Release:

```sh
cargo test --workspace --locked
./scripts/package-linux.sh
```

See [testing](testing.md) for the Docker image workflow and runtime checks.
