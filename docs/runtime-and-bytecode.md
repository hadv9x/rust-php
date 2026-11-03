# Runtime and bytecode

`php-runtime` owns the bytecode instruction set, runtime values, and virtual machine. It does not parse PHP source; `php-compiler` builds a `BytecodeProgram` that the runtime executes.

## Execution path

1. The compiler creates bytecode operations defined in [`bytecode/mod.rs`](../crates/php-runtime/src/bytecode/mod.rs).
2. [`vm/mod.rs`](../crates/php-runtime/src/vm/mod.rs) runs the instruction stream with a stack, local variables, static-property storage, and an instruction budget.
3. Specialized handlers implement arrays, control flow, functions, objects, and operators under [`vm/`](../crates/php-runtime/src/vm/).
4. `BytecodeProgram::execute()` returns captured output or a runtime error string.

## Runtime data

- [`values/value.rs`](../crates/php-runtime/src/values/value.rs) represents numbers, strings, booleans, arrays, objects, closures, and null.
- [`values/array.rs`](../crates/php-runtime/src/values/array.rs) stores ordered integer/string keys and append-index state.
- [`values/object.rs`](../crates/php-runtime/src/values/object.rs) stores class name and per-instance properties behind shared mutable handles.
- [`array_builtins/`](../crates/php-runtime/src/array_builtins/) dispatches supported array functions.

Object references are shared handles; arrays are cloned as values in this implementation. The runtime is a prototype and does not implement PHP's full copy-on-write/reference model, error levels, extensions, or native/JIT execution.

## Runtime errors

Compiler and runtime failures are returned as strings and printed by the CLI to stderr. The runtime applies an instruction budget to stop runaway recursion/loops. Diagnostics are intentionally simpler than PHP's full error and exception model.
