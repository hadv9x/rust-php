# References

Functions, methods, and closures can declare a parameter by reference with `&$value` or `Type &$value`. The caller must pass a variable. The callee and caller use the same storage during the call, so nested calls observe updates immediately. Closures can also capture a variable by reference with `use (&$value)`. Reference assignment (`$a =& $b`) supports variable aliases, binding a variable to an array element, and binding an array element to a variable or another nested array element, including computed keys. Array literals accept reference values in packed and keyed entries, and `foreach` supports by-reference value iteration over a variable-rooted array path; reference cells remain shared when an array containing them is copied.

```php
<?php
function increment(int &$value) {
    $value = $value + 1;
}

$count = 2;
increment($count);
$values = ["nested" => ["count" => 4]];
increment($values["nested"]["count"]);
echo $count, ":", $values["nested"]["count"];
```

See [`examples/reference-aliases.php`](../examples/reference-aliases.php) for alias assignment and live by-reference calls.
See [`examples/reference-returns.php`](../examples/reference-returns.php) for function, closure, and method reference returns.
See [`examples/array-reference-arguments.php`](../examples/array-reference-arguments.php) for nested array-offset parameters and array mutation through an offset. See [`examples/array-element-references.php`](../examples/array-element-references.php) for references embedded in array slots and preserved by array copies. See [`examples/foreach-reference.php`](../examples/foreach-reference.php) for by-reference foreach iteration.

This remains a partial reference implementation. By-reference arguments and return values can target variables, nested array offsets rooted at variables, public untyped instance properties, and static properties including nested array offsets. Direct alias assignment from a nested static-property offset preserves the cell stored in the static array. Assigning a reference-returning function, method, closure, or callable-array invocation with `=&` preserves the returned cell; callable-array references are covered for both an array stored in a variable and a literal callback array. Methods invoked through either form also receive writable variables by reference. Assigning the same call by value copies its current value. Direct references to typed properties and readonly properties now raise PHP-specific catchable `Error` diagnostics; overloaded typed-property reference reads check the current `__get()` reference value. Inaccessible property paths, the full set of overloaded-property reference behavior, and remaining callable forwarding/reference forms are incomplete. Unsetting a reference parameter detaches that local parameter binding while leaving the caller's variable intact.
