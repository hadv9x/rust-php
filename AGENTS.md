# Repository guidance

## PHP compatibility backlog

The active project objective is the compatibility backlog in [`docs/php-compatibility-backlog.md`](docs/php-compatibility-backlog.md). When working in this repository, continue that backlog autonomously across turns until every listed item is implemented and verified, or a concrete technical blocker is documented with evidence. Do not treat one small builtin or one passing test suite as completion of a category. Do not ask the user which backlog item to take next; select the next bounded, high-value slice and proceed.

For each slice:

1. Read the current implementation and the relevant compatibility notes before editing.
2. Carry behavior through syntax, AST, compiler, runtime, docs, and examples wherever that layer is involved.
3. Add focused regression coverage, then run the workspace tests, `cargo check --workspace --offline`, build the CLI, and run every `examples/*.php` file.
4. Update the backlog with what is complete, what remains, and any newly discovered gaps. Preserve unrelated behavior and repair regressions before moving on.
5. Report the remaining backlog honestly. Only call the overall objective complete when all ten categories are closed; otherwise continue on the next turn without asking the user to prompt again.

This backlog targets the gaps previously enumerated in this project. It does not imply complete compatibility with every PHP release or extension beyond those explicitly listed.
