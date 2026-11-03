# Namespaces

The current namespace subset supports multiple semicolon-style declarations or multiple bracketed `namespace Name { ... }` blocks, including `namespace { ... }` for global code. Simple and grouped class/function/constant imports with optional `as` aliases are supported, including mixed import kinds inside one grouped import. Constant import aliases preserve case and are resolved separately from class/function aliases. Each namespace declaration/block starts fresh import tables. Imports are applied to unqualified names and the first segment of qualified names, including type hints. Unqualified class, interface, trait, type, and function names otherwise use that namespace. Unqualified function calls fall back to a global user function or builtin when no namespaced function exists. A leading `\` marks a global fully qualified name.

Unqualified constant expressions are resolved in the active namespace and then fall back to a global constant. User-defined constant names retain their case. `__NAMESPACE__` expands to the active namespace name, or an empty string in global namespace code. `__LINE__` expands to the one-based source line containing the magic constant. `define()`, `defined()`, and `constant()` are available for supported constants.

```php
<?php
namespace Demo\Tools;

class Printer {}
$printer = new Printer();
echo count([1, 2]);
```

Run [`examples/namespaces.php`](../examples/namespaces.php) to see namespaced class/function resolution, imports, aliases, and global builtin fallback.
Run [`examples/namespace-constant-imports.php`](../examples/namespace-constant-imports.php) to see case-sensitive simple and grouped constant imports.
Run [`examples/namespace-mixed-imports.php`](../examples/namespace-mixed-imports.php) to see class, function, and constant aliases in one group.

## Current limits

- Bracketed and semicolon namespace styles cannot be mixed. Class, function, and constant imports use separate alias tables; duplicate aliases within an import kind are rejected.
- Name resolution is a subset; leading global names and namespace fallback rules need broader PHP compatibility checks.
