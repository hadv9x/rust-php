# Exceptions

The runtime supports explicit `throw`, typed `try`/`catch`, and `finally` blocks. A thrown `Throwable` propagates through function calls until a compatible catch block handles it. Throwing a scalar or an object that does not implement `Throwable` raises an `Error`. When a catch variable is declared, it receives the thrown object; catches match its declared class and known parent-class chain.

```php
<?php
try {
    throw new Exception("invalid input");
} catch (Throwable $error) {
    echo $error;
}
```

See [`examples/exceptions.php`](../examples/exceptions.php) for local and propagated exceptions, including creation and caller frames with source lines.

Catch types are matched against the thrown object's class and known parent-class chain; a mismatched nested catch continues to an outer handler. Built-in `Exception`, `Error`, and common subclasses (including `TypeError`, `ParseError`, `ArithmeticError`, `ValueError`, and JSON `JsonException`) can be constructed and expose `getMessage()`, `getCode()`, `getPrevious()`, `getFile()`, `getLine()`, `getTrace()`, and `getTraceAsString()`; named constructor arguments bind `message`, `code`, and `previous` and reject unknown or repeated names, and the previous value must be `null` or implement `Throwable`. Programs loaded with `compile_file()` record the canonical entry path. For supported `new` expressions, `getTrace()` includes creation and caller frames with statement source lines, omitting synthetic `{main}`; `getTraceAsString()` appends that frame. Top-level statements, named functions, methods, and closures from included files retain their canonical source path. Generated-code line gaps and locations for runtime-generated errors remain incomplete. User classes can extend these built-in exception classes and call the parent constructor; inherited native methods are recognized by `method_exists()` and `is_callable()`. User classes and interfaces cannot implement or extend `Throwable` directly. `finally` runs after normal completion, a handled or unhandled throw, and `return`, `break`, or `continue` leaving a protected body; try/finally without catch is supported. Runtime failures classified as `ValueError`, common argument `TypeError`/`ArgumentCountError`, division/modulo-by-zero `DivisionByZeroError`, negative-shift or integer-division-overflow `ArithmeticError`, calls to undefined functions or methods, method calls on non-objects, and conversion of non-stringable objects to strings (`Error`) become throwable objects and can be caught. Other runtime error sites and exact PHP messages remain incomplete.

See [`examples/builtin-exceptions.php`](../examples/builtin-exceptions.php) for built-in construction, inheritance checks, previous exceptions, and catch matching.

`ReflectionClass::getParentClass()` follows built-in exception parents, and `getInterfaceNames()`/`getInterfaces()` expose `Throwable` and inherited `Stringable` for exception classes.

A catch clause may list multiple class types with `|`, for example `catch (NetworkError|StorageError $error)`. A `try` body may also have several ordered `catch` clauses; the first matching clause handles the exception, and later clauses are skipped. Since PHP 8, the catch variable may be omitted when it is not needed. See [`examples/exception-union-catch.php`](../examples/exception-union-catch.php).

Trace frames for user methods preserve the declared class and method spelling in both `getTrace()` and `getTraceAsString()`, while internal lookup remains case-insensitive. See the trace example in [`examples/exceptions.php`](../examples/exceptions.php).

String conversion of built-in `Exception` and `Error` objects includes the class, message when non-empty, source location, and stack trace. A previous throwable is rendered first, followed by the current throwable with `Next`, matching PHP's chained exception layout. See [`examples/builtin-exceptions.php`](../examples/builtin-exceptions.php). Exact PHP behavior for all runtime-generated locations and custom Throwable implementations remains incomplete.
