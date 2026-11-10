# Release roadmap

This roadmap proposes release goals for `php-rust`. It is a planning document, not a promise of complete PHP compatibility. Each release must state its supported PHP feature profile, platforms, and known limitations in its release notes. The project remains experimental until those promises are deliberately narrowed and verified.

## Release principles

- Ship useful, reviewable milestones without waiting for every item in the compatibility backlog.
- Select features from a declared target PHP version/profile and real application needs. Do not imply compatibility with all of PHP or its extensions.
- Require a focused regression for each behavior change, update the relevant guide and example, and keep the compatibility backlog honest.
- Do not publish a release from a dirty or unreviewed checkout. Never reuse a published version or move a release tag.
- Keep the first binary support promise narrow: Linux x86_64 is the only packaged target today.

## 0.2.0 — Developer Preview

**Goal:** make the existing CLI and HTTP server practical to evaluate and install, while clearly labeling the runtime as experimental.

### Release gates

- [x] Review and commit the intended source and documentation changes; release preparation is committed as `e8652a9` on a clean working tree.
- [x] Prepare versioned release notes with the tested Rust toolchain, Linux x86_64 target, install/run instructions, and known limitations.
- [x] Confirm README, CLI/server guides, examples, and release assets agree about what is supported.
- [x] Establish a deterministic CI example set and document that live database/application checks require separate setup; all 124 examples pass locally.
- [x] Establish a formatting baseline, record the Rust 1.93.1 Clippy baseline (173 warning entries), and add format/lint checks to CI with an explicit non-blocking warning policy.
- [x] Decide MSRV policy: no minimum Rust version is promised for 0.2.0; Rust 1.93.1 is pinned for CI/release reproducibility.
- [x] Build the Linux x86_64 archive and pinned-toolchain Docker image locally; verify the installer, CLI version, Docker version label, and Docker CLI.
- [ ] Run the release workflow from the exact tagged commit and verify downloaded archive checksums and installation instructions.

### Scope

This release can include stable improvements already present in the repository. It does not need to close whole compatibility categories. Avoid expanding platform claims until those targets build and pass tests in CI.

## 0.3.0 — Compatibility Preview

**Goal:** improve compatibility for common PHP code and make the project's target precise.

### Required planning decision

- [ ] Declare the target PHP language version/profile, supported extensions or built-in API families, and the method used to check behavior against PHP. Keep unsupported features explicit.

### Candidate implementation priorities

1. Close high-impact runtime errors and exception behavior, including useful source locations and traces.
2. Improve common array, reference, OOP, and type behavior based on regression cases and representative application code.
3. Choose standard-library additions from a published API inventory; preserve consistent parameter names, named-argument binding, callback dispatch, and `function_exists()` behavior.
4. Continue parser and `Traversable` work where it blocks those use cases.

### Release gates

- [ ] Each selected compatibility slice has focused regression coverage and updated docs/examples.
- [ ] Run the workspace tests, offline check, CLI build, and the runnable PHP example set; record any excluded examples and why.
- [ ] Publish a compatibility matrix showing supported, partial, and unsupported behavior for the selected profile.

## 0.4.0 — Application Preview

**Goal:** evaluate a deliberately small set of realistic applications and deployment patterns.

### Candidate implementation priorities

- [ ] Select representative applications and pin their versions and setup steps.
- [ ] Turn each chosen application into a repeatable smoke test where licensing and infrastructure permit.
- [ ] Implement only the framework, Composer, HTTP, upload, and database requirements found in those targets; document unsupported framework requirements.
- [ ] Measure server performance on representative routes under controlled load and publish the environment and results.
- [ ] Document deployment boundaries, including reverse-proxy assumptions, session storage behavior, worker limits, and database support.

### Release gates

- [ ] Selected application smoke tests pass in the documented environment.
- [ ] Known failures and unsupported application requirements are listed in release notes.
- [ ] Load and resource-limit behavior has a reproducible measurement; no general production-capacity claim is made from a trivial route benchmark.

## 1.0.0 — Stable within a declared profile

**Goal:** make a bounded compatibility and stability commitment, not claim full PHP replacement.

### Release gates

- [ ] Publish a versioned compatibility profile that names the PHP target, available APIs/extensions, supported OS/architectures, and database backends.
- [ ] Define semantic-versioning expectations for the CLI, server configuration, runtime behavior, and any published Rust crate APIs.
- [ ] Document deprecation and security-reporting policies and a support window for stable releases.
- [ ] Run CI across every advertised target and database integration used in the support promise.
- [ ] Ensure release packages, checksums, source, and release notes are reproducible and verified from the tagged commit.
- [ ] Resolve all release-blocking issues against the declared profile. The broader compatibility backlog may remain open outside that profile.

## Work that should not block every release

These are substantial backlog items. Schedule them when the declared profile or selected application tests require them, rather than treating them as universal release gates:

- Resumable asynchronous VM execution and non-blocking database/filesystem I/O.
- Complete PHP Manual or extension parity.
- Full PDO 8.5 surface, native streaming LOBs, and cursor-backed results for every driver.
- Complete MongoDB library/API parity.
- macOS, Windows, or non-x86_64 release packages before those targets are selected and tested.

## Release checklist

Before tagging any version:

1. Confirm the release scope, compatibility profile, and version bump across workspace manifests.
2. Review the Git diff and confirm no unintended local changes are included.
3. Run the documented CI and release checks for that version; record external-service tests that were unavailable.
4. Update the changelog/release notes, compatibility matrix, and relevant guides.
5. Commit and push the release commit, then create a matching annotated `vX.Y.Z` tag as described in [release process](release-process.md).
6. Verify the GitHub Actions release, archive checksums, Linux installation, and Docker image before announcing availability.
