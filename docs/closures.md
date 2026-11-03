# Closures

Anonymous functions can declare parameters, an optional return type, and explicit `use (...)` captures. In a non-static class method, anonymous functions also bind `$this` automatically and retain the enclosing class scope, so they can access private members. Arrow functions (`fn (...) => ...`) capture referenced outer variables by value automatically, excluding variables shadowed by parameters; they also capture `$this` in non-static methods. Captures are copied when the closure is created; changing a captured variable inside the closure updates the closure's private captured value for later calls.

```php
<?php
$factor = 3;
$multiply = function ($value) use ($factor) {
    return $value * $factor;
};
echo $multiply(4);
```

Closures can be passed to `array_map`, `array_filter`, `array_reduce`, `array_all`, `array_any`, `array_find`, and `array_find_key`. See [`examples/closures.php`](../examples/closures.php) for closure calls, persistent by-value capture state, and array callbacks.

Static anonymous and arrow functions are supported; they keep lexical captures but do not bind `$this`. Static closures cannot capture or use `$this`. By-reference parameters and explicit `use (&$value)` captures are supported; see [`examples/reference-aliases.php`](../examples/reference-aliases.php). Array builtins accept simple object and static-method callable arrays. `array_walk` and `array_walk_recursive` support a by-reference first callback parameter; other array callback functions do not.
