# Development guide

## Build and run

```sh
cargo check --workspace
cargo run -p php-cli -- examples/hello.php
cargo run -p php-cli -- examples/arrays.php
cargo run -p php-cli -- examples/loops.php
cargo run -p php-cli -- examples/switch.php
cargo run -p php-cli -- examples/logical-operators.php
cargo run -p php-cli -- examples/functions.php
cargo run -p php-cli -- examples/classes.php
```

Examples are executable documentation: when adding a supported function or syntax feature, add a small example showing how to use it.

## Follow a feature through the source

| Feature | Parse / AST | Generate bytecode | Execute |
| --- | --- | --- | --- |
| Array literals/reads | `php-syntax/src/parser/arrays/mod.rs`, `ast/expressions/mod.rs` | `php-compiler/src/codegen/arrays/mod.rs` | `php-runtime/src/vm/arrays.rs` |
| Array builtins | `php-syntax/src/parser/expressions/mod.rs` | `php-compiler/src/codegen/expressions/mod.rs` | `php-runtime/src/array_builtins/` and `vm/functions.rs` |
| Loops | `php-syntax/src/parser/loops/mod.rs`, `ast/statements/mod.rs` | `php-compiler/src/codegen/loops/mod.rs` | `php-runtime/src/vm/control.rs` |
| Functions | `php-syntax/src/parser/functions/mod.rs`, `ast/functions/mod.rs` | `php-compiler/src/codegen/declarations/functions/mod.rs` | `php-runtime/src/vm/functions.rs` |
| Classes/OOP | `php-syntax/src/parser/declarations/mod.rs`, `ast/classes/mod.rs` and related ASTs | `php-compiler/src/codegen/declarations/classes/` | `php-runtime/src/vm/objects.rs`, `vm/functions.rs` |

## Change workflow

1. Find the feature in the table above and read its parser, compiler, and runtime files.
2. Update the AST/bytecode representation only when the existing nodes/instructions cannot express the behavior.
3. Add comments for new functions and non-obvious control-flow or compatibility rules.
4. Add/update a PHP example and the related document under `docs/`.
5. Run `cargo check --workspace` and execute the relevant example(s).

The project currently emits internal VM bytecode. It does not yet emit native machine code or claim compatibility with general PHP applications.
