# Syntax and parser

`php-syntax` converts PHP source text into a `Program` syntax tree. It does not generate bytecode or execute code.

## Parsing pipeline

```text
source text → lexer tokens → parser → AST
```

1. [`lexer/mod.rs`](../crates/php-syntax/src/lexer/mod.rs) recognizes tokens and records each token's source offset.
2. [`parser/`](../crates/php-syntax/src/parser/) consumes tokens and reports errors with line and column information.
3. [`ast/`](../crates/php-syntax/src/ast/) defines expressions, statements, declarations, and the program root.

For common syntax, start with:

- [`parser/arrays/mod.rs`](../crates/php-syntax/src/parser/arrays/mod.rs) for array literals.
- [`parser/loops/mod.rs`](../crates/php-syntax/src/parser/loops/mod.rs) for loop headers and bodies.
- [`parser/control/mod.rs`](../crates/php-syntax/src/parser/control/mod.rs) for switch/case/default parsing.
- [`parser/functions/mod.rs`](../crates/php-syntax/src/parser/functions/mod.rs) for function and method signatures.
- [`parser/declarations/mod.rs`](../crates/php-syntax/src/parser/declarations/mod.rs) for classes, interfaces, traits, and top-level assembly.
- [`parser/expressions/mod.rs`](../crates/php-syntax/src/parser/expressions/mod.rs) for operators, calls, member access, and expression precedence.
- [`parser/statements/mod.rs`](../crates/php-syntax/src/parser/statements/mod.rs) for assignments, branches, returns, and other statements.

## Recognized syntax

The current frontend supports PHP tags, line/hash comments, scalar literals, top-level `const` declarations, global/namespaced constant references, and contextual `__CLASS__`/`__FUNCTION__`/`__METHOD__`; variables, `global` and function-local `static` declarations with first-call expression initializers, variable variables using `$$name` and `${expression}`, variable and nested array-element aliases (`=&`), assignment expressions and compound assignments on variables, nested array offsets, instance properties, and static properties, plus prefix/postfix increment and decrement on variables, variable variables and their nested offsets, string offsets, instance properties, static properties, dynamic object properties, and dynamic-class static properties with array offsets; arrays with packed/keyed reference entries and unpacking, plus call-site argument unpacking from arrays, positional, literal-keyed, computed-keyed, and nested `list()`/short-array destructuring, including array-offset and object-property destinations with computed or nested receivers; logical/null-coalescing/ternary and strict `match` expressions; multiple semicolon or bracketed namespace blocks with simple and grouped class/function imports; positional and variadic parameters, including by-reference variadics; by-reference function, method, constructor, and closure parameters; by-value and by-reference closure captures; lazy generator `yield` and `yield from` for arrays/generators with explicit/automatic keys, send/throw delegation, and resumable `foreach`; explicit `throw`, typed `try`/`catch`, and `finally`; closure invocation, arrow functions, and static closures; declarations; conditionals; loops; brace and alternate-syntax `switch`/`case`/`default`; and a growing OOP subset. Short-circuit expressions evaluate only the branches PHP would evaluate. See the examples directory for runnable samples.

Both symbolic `&&`/`||` and word-form `and`/`or`/`xor` are supported. The word operators retain PHP's lower precedence than assignment and their relative precedence (`and` above `xor` above `or`). Integer and string bitwise `&`, `|`, and `^`, integer shifts `<<`/`>>`, unary complement `~`, and their compound assignment forms follow PHP precedence. String bitwise operations are bytewise; results that are not valid UTF-8 are lossily decoded by the runtime's string representation. Negative shift counts are runtime errors. Arithmetic includes `%` and `%=`; both operands are converted to integers, and modulo by zero is a runtime error. See [`examples/bitwise-operators.php`](../examples/bitwise-operators.php) and [`examples/bit-shifts.php`](../examples/bit-shifts.php).

Strict comparisons `===` and `!==` compare scalar types as well as values. Arrays must have matching keys and values in the same order; object and closure identity uses the same runtime handle. Loose `==`/`!=` follows PHP 8 scalar coercion, compares arrays by key/value regardless of order, and compares user objects of the same class by their properties. Internal/built-in object comparison and PHP's full recursive-comparison error behavior are not implemented. See [`examples/strict-equality.php`](../examples/strict-equality.php) and [`examples/loose-equality.php`](../examples/loose-equality.php).

Scalar and container casts `(int)`/`(integer)`, `(float)`/`(double)`/`(real)`, `(string)`/`(binary)`, `(bool)`/`(boolean)`, `(array)`, and `(object)` are supported. Numeric string casts consume a leading numeric prefix; nonnumeric strings become zero for numeric casts. Casting arrays to strings returns `"Array"`; object-to-string casts invoke `__toString()` and require it to return a string. `(array)` on a closure is unsupported. Casting a user object to an array flattens its properties under plain string keys; PHP visibility-mangled keys and declared property order are not retained by the current object representation. See [`examples/scalar-casts.php`](../examples/scalar-casts.php).

PHP's full grammar is not implemented. Basic union, intersection, and parenthesized DNF type declarations are supported for parameters, returns, and properties, with nominal class/interface checks. Callable-array invocations support by-reference arguments and reference-returning instance/static methods, including literal arrays; first-class callable syntax accepts literal instance and static callable arrays. Remaining syntax gaps include `goto`. `declare(strict_types=1)` enforces scalar argument types at compiled function, method, closure, callable, and constructor call sites, and scalar return types for functions, methods, and closures, allowing `int` for `float`. Statically expanded includes retain each file's mode; runtime/dynamic includes and remaining typing edge cases are incomplete. Argument unpacking and `foreach` now cover arrays, generators, PDO statements, and user `Iterator`/`IteratorAggregate` objects, subject to the remaining limits in the compatibility backlog. `match` evaluates its subject once, compares conditions strictly from left to right, and supports a default arm; a missing match throws a catchable `UnhandledMatchError`, with simplified message text. Union catch types are supported. By-reference function, method, and closure returns are supported for variables, variable-rooted array offsets, and public untyped instance properties. A feature must exist in the lexer, parser, AST, compiler, and runtime before a script using it can run.

See [`examples/constants.php`](../examples/constants.php) for namespaced constants, constant references, and array-valued constants.
See [`examples/magic-constants.php`](../examples/magic-constants.php) for contextual class, function, method, and closure values.

## Adding a syntax feature

1. Add or reuse tokens in [`lexer/token.rs`](../crates/php-syntax/src/lexer/token.rs) and recognize new characters in the lexer.
2. Add an AST node under `ast/` when the feature needs a distinct representation.
3. Parse it in the feature-specific parser file and preserve source-positioned diagnostics.
4. Add an example under `examples/` so the syntax is visible and runnable.
5. Continue the feature through compiler and runtime; parsing alone does not make the syntax executable.

Exponentiation uses PHP's right-associative `**` operator. It binds more tightly than unary minus, so `-2 ** 2` evaluates as `-(2 ** 2)`. The compound assignment form `**=` is supported for variables, nested array offsets, and instance or static properties. See [`examples/power-operators.php`](../examples/power-operators.php).
See [`examples/intersection-dnf-types.php`](../examples/intersection-dnf-types.php) for intersection checks and parenthesized DNF declarations.

`foreach` over a user `Iterator` preserves any value returned by `Iterator::key()` in the key variable, including arrays and objects. `iterator_count()` counts without converting keys; `iterator_to_array()` applies scalar array-key coercions and warns/skips object or array keys when preserving keys, while `preserve_keys: false` reindexes values without reading keys. Generators and `yield from` also preserve non-scalar iterator keys. See [`examples/generators.php`](../examples/generators.php).
