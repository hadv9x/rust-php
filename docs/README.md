# Documentation index

Use this page to find the documentation for each part of the project.

## Language features

- [PHP manual coverage audit](php-manual-coverage.md) — manual-wide missing feature families and audit scope.
- [Syntax and parser](syntax-and-parser.md) — how source becomes tokens and AST nodes, and which syntax is currently recognized.
- [Control flow and loops](control-flow.md) — `if`, `switch`, `match`, `while`, `do...while`, `for`, `foreach`, `break`, and `continue`.
- [Functions](functions.md) — user functions, parameters, supported references, return types, calls, and current restrictions.
- [Variables and scope](variables.md) — variable storage, `global`, assignment expressions, and remaining scope gaps.
- [Closures](closures.md) — anonymous functions and by-value captures.
- [Namespaces](namespaces.md) — file-level namespace declarations and name resolution.
- [References](references.md) — supported by-reference function parameters and gaps.
- [Exceptions](exceptions.md) — `Throwable`-based `throw`, `try`/`catch`, and `finally`.
- [Object-oriented features](oop-compatibility.md) — classes, inheritance, interfaces, traits, properties, constants, magic methods, and gaps.
- [Array compatibility](array-compatibility.md) — array syntax, behavior, implemented functions, and compatibility gaps.

## Internals and use

- [PHP compatibility backlog](php-compatibility-backlog.md) — the active ten-area implementation checklist and verification protocol.

- [Compiler architecture](compiler-architecture.md) — project layers, bytecode flow, and design references.
- [Runtime and bytecode](runtime-and-bytecode.md) — VM execution, values, arrays, objects, and errors.
- [CLI and installation](cli-and-installation.md) — commands, local installation, and Linux package usage.
- [Build, install, and test](testing.md) — reproducible build/test commands and an installed-server HTTP smoke test and load script.
- [Web server](web-server.md) — HTTP hosting, request globals, front-controller behavior, and current limits.
- [Database connections](database.md) — configured pools, SQL drivers, and the current PDO surface.
- [Development guide](development.md) — build commands and a feature-to-source-file map.

The [README](../README.md) also contains a compact feature map linking syntax, compiler, and runtime files.
