# Open-source maintenance checklist

This page records what repository automation provides and which GitHub settings still need owner action. Workflow files alone cannot enforce repository rules such as required reviews or branch protection.

## Automated in this repository

- [x] Rust workspace tests, checks, and release build on pushes and pull requests: [CI workflow](../.github/workflows/ci.yml).
- [x] Scheduled and pull-request Cargo dependency advisory scan: [dependency audit](../.github/workflows/dependency-audit.yml).
- [x] Weekly Dependabot updates for Cargo dependencies and GitHub Actions: [Dependabot configuration](../.github/dependabot.yml).
- [x] Tag-triggered Linux/Docker/source release with checksums: [release workflow](../.github/workflows/release.yml) and [release guide](release-process.md).
- [x] MIT license, contribution guide, security reporting instructions, and pull request checklist.

## Not yet enforced by CI

- [x] `rustfmt` formatting check and Clippy CI gate. Formatting passes with Rust 1.93.1. Clippy runs on all workspace targets; its 173 emitted warning entries on the initial Rust 1.93.1 baseline are informational, while command/compilation failures fail CI.
- [x] Run every top-level `examples/*.php` file in CI using the release CLI in a fresh process. Live database and application-server integration checks remain separate because they need external services or setup.
- [ ] Cross-platform build/test matrix. The release package currently targets Linux x86_64; support targets should be selected and tested before advertising them.
- [x] MSRV decision: 0.2.0 does not promise a minimum Rust version; CI and release builds pin Rust 1.93.1 for reproducibility. Revisit MSRV before making a stable 1.0 compatibility promise.
- [ ] Provenance attestations and releases for targets beyond Linux x86_64.

## GitHub repository settings for maintainers

In **Settings → Rules → Rulesets**, protect `master` (and `main` if that becomes the default branch):

1. Require pull requests before merging and require at least one approval when the project has multiple maintainers.
2. Require the `Test workspace (Rust 1.93.1)` and `Audit Rust dependencies` checks when applicable. The dependency audit check is only emitted for dependency changes or its scheduled/manual run, so avoid making a conditional check permanently required unless the workflow is adjusted to always emit it.
3. Block force pushes and branch deletion on the protected branch.
4. Enable Dependabot security updates and dependency graph. Enable secret scanning and push protection when available for the repository plan.
5. Review **Settings → Actions → General** and keep workflow token permissions read-only by default.

GitHub supports required status checks and branch rules through rulesets; maintainers must enable these settings in the repository because they cannot be committed as workflow YAML. See [GitHub's ruleset status-check documentation](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets).

## Workflow security

Workflows declare read-only `GITHUB_TOKEN` permissions, pin third-party actions to full commit SHAs, and do not run privileged `pull_request_target` jobs. Dependabot is configured to propose updates to those action pins. GitHub recommends SHA pinning because tags can move; see [GitHub Actions security hardening](https://docs.github.com/en/code-security/tutorials/secure-your-organization/protect-against-threats).
