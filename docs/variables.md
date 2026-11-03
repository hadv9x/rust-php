# Variables and scope

Variables are stored in the current VM frame. The top-level frame is the program's global scope. A function or method gets its own local frame and can bind selected names to the shared global table with `global`.

```php
<?php
$count = 0;
function incrementCount() {
    global $count;
    $count++;
}
incrementCount();
echo $count;
```

Global bindings remain live across nested function calls. Assignment, array mutation, increment/decrement, `unset`, and by-reference arguments use the bound storage directly. Simple aliases such as `$a =& $b` share storage. `$$name` and `${expression}` are supported for variable-variable reads; both forms can be used for simple assignments. Variable assignments, supported compound assignments, nested array-path, instance-property, and static-property assignments (including `??=`) also work inside expressions.

Function-local `static` variables persist between calls in one program execution. Recursive calls share the same static slot; a method's static local is shared across instances, and each closure object gets its own static locals. Initializers support arbitrary expressions (PHP 8.3+), including function calls and local parameters. The first successful initialization wins; if an initializer throws, a later call retries it. Recursive re-entry may evaluate the expression more than once.

`compact()` collects existing names from the active scope, including names nested in input arrays, and omits undefined variables. `extract()` imports array keys into the active scope and supports `EXTR_OVERWRITE` (0), `EXTR_SKIP` (1), `EXTR_PREFIX_SAME` (2), `EXTR_PREFIX_ALL` (3), `EXTR_PREFIX_INVALID` (4), `EXTR_PREFIX_IF_EXISTS` (5), and `EXTR_IF_EXISTS` (6), passed as numeric flags. `EXTR_REFS` (256) links imported variables to their source array elements. The prefix modes require a nonempty string prefix; non-ASCII identifier validation is not implemented.

`$GLOBALS` exposes the current named global variables as an array in insertion order. Ordinary user function frames also preserve parameter and local-variable insertion order in `get_defined_vars()`. Reading and changing entries, nested offsets, appending, unsetting entries, binding entry references with `=&`, and passing an individual array entry to a by-reference mutator update the global symbol table, including from functions. Assigning a replacement array directly to `$GLOBALS` is rejected. Passing the whole table directly to a known by-reference array mutator, a declared user function, or a statically resolvable user method is also rejected during compilation. See [`examples/globals.php`](../examples/globals.php).

## Current limits

- Callable-array references are not implemented. Simple variable aliases, array-slot references, by-reference function/method/constructor/closure parameters and returns, reference captures, public untyped property arguments/returns, and `extract(..., EXTR_REFS)` bindings are supported.
- Deeply nested variable-variable forms and dynamic array l-values are not implemented. Undeclared dynamic object properties can be read, written, unset, and detected by `property_exists()`; PHP's dynamic-property deprecation warnings are not emitted.
- The standard superglobal arrays (`$_GET`, `$_POST`, `$_COOKIE`, `$_REQUEST`, `$_FILES`, `$_SERVER`, `$_ENV`, and `$_SESSION`) are initialized as empty arrays for standalone runs; the HTTP server fills in request-backed values. They remain accessible from function and method scopes. `$argc` and `$argv` are populated by the CLI binary; direct runtime API executions leave them undefined. Dynamic object types, dynamic callables, unpacking, internal methods, and additional reference-taking built-ins still need a compile-time `$GLOBALS` audit. Full undefined-variable diagnostics are also missing.

See [`examples/superglobals.php`](../examples/superglobals.php) for standalone initialization and shared access from a function.
