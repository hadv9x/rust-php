# Control flow and loops

Control-flow syntax is split by feature. Loop parsing is in [`php-syntax/src/parser/loops/mod.rs`](../crates/php-syntax/src/parser/loops/mod.rs), switch parsing is in [`php-syntax/src/parser/control/mod.rs`](../crates/php-syntax/src/parser/control/mod.rs), and AST nodes are in [`ast/statements/mod.rs`](../crates/php-syntax/src/ast/statements/mod.rs). Loop bytecode is in [`php-compiler/src/codegen/loops/mod.rs`](../crates/php-compiler/src/codegen/loops/mod.rs), switch dispatch is in [`php-compiler/src/codegen/control/mod.rs`](../crates/php-compiler/src/codegen/control/mod.rs), and VM jumps/iteration are in [`php-runtime/src/vm/control.rs`](../crates/php-runtime/src/vm/control.rs).

## Supported constructs

- `if` / `else`
- `while`
- `do...while`
- `for`, including comma-separated initializer and update clauses
- `foreach` over arrays, generators, PDO statements, and user `Iterator`/`IteratorAggregate` objects, with optional keys and by-reference array value binding (`&$value`)
- `switch` with `case`, one `default` arm, fall-through, and `break`, using braces or `: ... endswitch;`
- `match` expressions with strict comparison, multiple condition values, one default arm, and catchable `UnhandledMatchError` when nothing matches
- `break` and `continue`, including positive numeric nesting levels
- Prefix/postfix increment and decrement on variables, nested array offsets, instance properties, and static properties
- Prefix/postfix increment on ASCII alphanumeric strings, including carry across digits and letter ranges (`z` to `aa`, `9` to `10`); numeric strings use numeric arithmetic and empty strings increment to `"1"`
- Braced and single-statement loop bodies

Run [`examples/loops.php`](../examples/loops.php) for loops and numeric loop targets. See [`examples/foreach-reference.php`](../examples/foreach-reference.php) for by-reference iteration and [`examples/string-increment.php`](../examples/string-increment.php) for alphanumeric string increments. Run [`examples/switch.php`](../examples/switch.php) for brace syntax and [`examples/switch-alternate.php`](../examples/switch-alternate.php) for alternate syntax.

```php
for ($i = 0; $i < 4; $i++) {
    if ($i == 1) {
        continue;
    }
    echo $i, "\n";
}
```

`break 2` and `continue 2` target the second active breakable construct outward. Levels may be expressions evaluated at runtime; they must produce a positive integer no greater than the active loop/switch depth. A computed invalid level produces a runtime error. Literal invalid levels are rejected during compilation. `continue` inside a switch exits that switch, matching PHP's behavior for `continue 1`.

## Current limits

- `for` initializer/update clauses support simple variable assignments and expressions; assignment expressions are not general PHP expressions.
- PHP 8.3 deprecation diagnostics for empty, nonnumeric, and non-alphanumeric increment/decrement operations are not emitted. Decrement on nonnumeric strings is a no-op except that an empty string becomes `-1`.
- User iterator support follows the `Iterator` methods and resolves nested `IteratorAggregate::getIterator()` results. Aggregate methods must return a supported Traversable value. By-reference iteration accepts array paths rooted at variables; offset expressions are evaluated once.
- Switch case expressions use the parser's supported expression subset.
- `match` compares with PHP strict identity and evaluates the subject once. The `UnhandledMatchError` message is simplified and does not include PHP's full value rendering.
- Loop conditions use the expression operators supported by this frontend, which is less than PHP's full operator set.
