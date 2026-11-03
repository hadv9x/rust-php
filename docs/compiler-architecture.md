# Compiler architecture and references

## Pipeline used here

```text
PHP source
  -> lexer (tokens)
  -> parser (AST)
  -> bytecode compiler
  -> bytecode VM
```

Each stage has its own module so syntax parsing does not also emit VM instructions. The AST is the boundary between the PHP grammar and the runtime's instruction set.

Within the syntax crate, AST nodes are grouped into domain folders under `src/ast/` for expressions, statements, functions, classes, interfaces, traits, and programs. Parsing uses domain folders under `src/parser/` for expressions, arrays, loops, statements, functions, and class/interface/trait declarations, with shared parser state in `parser/mod.rs`; tokenization lives under `lexer/`. Compiler generation uses feature directories under `php-compiler/src/codegen/`, with each domain's implementation in its `mod.rs`; declaration code is grouped under `codegen/declarations/`, with class metadata, compatibility checks, and property metadata under `classes/`. Runtime data types are grouped under `php-runtime/src/values/`, bytecode instructions under `bytecode/`, VM instruction handling under `vm/`, and array builtins under `array_builtins/`. Each directory keeps related implementation files together, and the README feature map links each common feature across these layers.

## What we borrowed from established compilers

- Go's compiler separates parsing and syntax trees from later compiler representations and backend work. We follow that separation at a smaller scale: `php-syntax` (lexer/parser/AST), then `php-compiler` (bytecode generation), then `php-runtime` (VM execution), with `php-cli` as the executable entry point. See the [Go compiler overview](https://go.dev/src/cmd/compile/README).
- Rust's compiler uses several intermediate representations (AST, HIR, MIR, LLVM IR), with each representation useful to a later phase. We use a PHP AST and VM bytecode now; adding multiple lower-level IRs before the language needs them would add complexity without immediate value. See the [rustc compiler overview](https://rustc-dev-guide.rust-lang.org/overview.html).
- PHP's Zend Engine compiles PHP source to opcode arrays and executes them in a VM. That is the closest fit for an implementation aiming to run PHP with dynamic semantics and a standalone CLI. See the [PHP source compiler and opcode definitions](https://github.com/php/php-src/blob/master/Zend/zend_compile.h) and [Zend VM opcodes](https://github.com/php/php-src/blob/master/Zend/zend_vm_opcodes.h).

## Planned layers

1. Carry source spans through AST nodes and expose structured diagnostics from the public compiler API.
2. Grow AST and parser coverage for PHP expressions, arrays, functions, and classes; track array compatibility in `docs/array-compatibility.md`.
3. Add semantic/name resolution and runtime error behavior.
4. Expand VM instructions and PHP core functions, then use PHP's compatibility tests as coverage grows.
5. Profile real PHP workloads before adding bytecode optimization or a native/JIT backend.

The current compiler emits internal VM bytecode. It does not yet generate native machine code, and its PHP grammar/runtime coverage is intentionally small.

Loop and switch syntax/bytecode are split into dedicated modules. The current control-flow subset includes `if`, `switch` with loose case matching and fall-through, `while`, `do...while`, `for` with comma-separated initializer/update clauses, `foreach`, prefix/postfix increment/decrement on variables, and numeric `break N`/`continue N` targets across nested loops and switches. Invalid levels and `break`/`continue` outside an active breakable construct are rejected during compilation.

The current object subset includes class declarations, single inheritance, scalar instance/static property defaults, typed scalar, nullable, and class-typed properties, readonly instance properties initialized once in the declaring constructor, constructor property promotion, `new ClassName(...)` and `new static()`, inherited instance properties and methods, `__construct`, `parent::method()` calls, instance method calls, static methods via `ClassName::method()`, `self::method()`, and late-static `static::method()`, static property reads/writes via `ClassName::$property`, `self::$property`, and `static::$property`, scalar class constants with visibility and `final`, abstract/final classes and methods, and runtime public/protected/private access checks for properties, constructors, methods, and constants. It also supports interfaces with public constants, compile-time method/arity/visibility/parameter/return-type checks, inherited interface obligations and constants, single and multiple interface inheritance, trait method/property composition with `insteadof` conflict selection and `as` aliases/visibility changes, object cloning with `__clone()`, `__get()`/`__set()` property interception, `isset($object->property)` and `unset($object->property)` with `__isset()`/`__unset()` dispatch, `__call()`/`__callStatic()` missing-method dispatch, `__toString()` during echo, and `$this`. Static property state is shared within one program execution and reset for each new execution. Objects are shared mutable handles in the VM so property changes remain visible through aliases.

Enums retain their unit or backed declaration kind; cases are singleton objects with `name` and, for backed enums, `value`, and `Enum::cases()` returns them in declaration order. Backed enums support `from()` and `tryFrom()` with basic weak scalar coercion; enum construction, case cloning, and case-property writes/unsets are rejected. Invalid `from()` values throw a catchable `ValueError`. Implicit `UnitEnum`/`BackedEnum` interfaces, enum reflection, strict typing, and full declaration validation remain incomplete. Other OOP gaps include complete late-static return types, trait inheritance edge cases, distinct storage for same-named private parent/child properties, autoloading, destructors, most magic methods beyond `__clone()`, `__get()`, `__set()`, `__isset()`, `__unset()`, `__call()`, `__callStatic()`, and `__toString()`; nested overloaded-property `isset()` cases remain incomplete. Basic union, intersection, and DNF types are supported for parameters, returns, and properties. Simple-variable references are supported for function/method/constructor/closure parameters and returns and for closure captures; computed array-path arguments and returns work for variable-rooted array offsets. Public untyped instance-property arguments and returns work by reference; typed, readonly, or inaccessible property references remain incomplete. `iterable` type checks accept arrays; Traversable objects are not implemented. This implementation does not claim PHP language or runtime compatibility.
