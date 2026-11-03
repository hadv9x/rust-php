# Object-oriented features

This document describes the OOP subset implemented by this project. It is a status guide, not a claim of full PHP compatibility.

User classes can declare non-static, zero-argument `__destruct()` methods with public, protected, or private visibility; constructors and destructors cannot declare return types. The runtime invokes remaining registered destructors in reverse object-creation order during script shutdown. Explicit `unset()` invokes a destructor when it removes the last variable reference from a variable, nested array element, object-property array element, or static-property array element. Reassignment of variables, direct object/static properties, and array slots invokes destruction when the overwritten value loses its last reference, including aliases held by copied arrays. Property-hook lifetime details and cycle collection remain incomplete.

Anonymous classes support constructor arguments, properties, methods, `extends`, and `implements`. Attributes in `new #[...] class {}` declarations are retained and available through `ReflectionClass` when given the object. Anonymous-class `use (...)` captures, traits, and full PHP class-name/reflection behavior remain incomplete.

## Where the implementation lives

- Parser: [`php-syntax/src/parser/declarations/mod.rs`](../crates/php-syntax/src/parser/declarations/mod.rs) and [`parser/functions/mod.rs`](../crates/php-syntax/src/parser/functions/mod.rs).
- AST: [`ast/classes/mod.rs`](../crates/php-syntax/src/ast/classes/mod.rs), [`ast/interfaces/mod.rs`](../crates/php-syntax/src/ast/interfaces/mod.rs), [`ast/traits/mod.rs`](../crates/php-syntax/src/ast/traits/mod.rs), and [`ast/functions/mod.rs`](../crates/php-syntax/src/ast/functions/mod.rs).
- Compiler checks and class assembly: [`php-compiler/src/codegen/declarations/classes/`](../crates/php-compiler/src/codegen/declarations/classes/mod.rs), with separate inheritance compatibility and metadata validation helpers.
- Runtime object/member behavior: [`php-runtime/src/values/object.rs`](../crates/php-runtime/src/values/object.rs), [`vm/objects.rs`](../crates/php-runtime/src/vm/objects.rs), and [`vm/functions.rs`](../crates/php-runtime/src/vm/functions.rs).
- Runnable coverage: [`examples/classes.php`](../examples/classes.php).

## Implemented subset

- Classes, single class inheritance, `new`, constructors, instance/static methods, `self::`, `parent::`, and late-static `static::` for method/property/constant lookup and `new static()`.
- Anonymous classes with constructor arguments, properties, methods, one parent, and implemented interfaces.
- Public/protected/private access checks for properties, methods, constructors, and class constants.
- Instance/static properties, scalar defaults, scalar/nullable/class property type checks, readonly instance properties, and constructor property promotion.
- Basic PHP 8.4 instance property `get`/`set` hooks and asymmetric setter visibility are parsed and executed; hook validation, backed-property edge cases, and complete PHP property-hook semantics remain incomplete.
- Traits can declare instance and static properties; each class using a static trait property gets an initialized class storage slot, and trait methods can read/update it through `self::$property`.
- Optional method and constructor parameters with scalar literal defaults.
- Scalar class constants, visibility, final constants, and interface constants.
- Abstract/final classes and methods.
- Interfaces with inherited method obligations and supported parameter/return variance checks. The built-in `Countable` contract requires a public instance `count()` method with no required arguments; a declared return type must be `int`.
- `instanceof` checks an object against a named class/interface or runtime class-name variable, including parent classes and implemented interfaces; `self`, `parent`, and late-bound `static` are supported in methods.
- `get_class()` and `get_parent_class()` inspect object/class names and use the active instance/static class when called without an argument inside a method.
- Class, interface, enum, method, property, method-parameter, named-function, and anonymous-class attributes with constant positional/named arguments are retained and exposed through the corresponding reflection APIs, including `ReflectionFunction::getAttributes()`, `ReflectionMethod::getParameters()`, and `ReflectionParameter::{getName,getPosition,getAttributes}`. `ReflectionAttribute::{getName,getArguments,getTarget,isRepeated,newInstance}` and `Attribute::TARGET_*`, `TARGET_ALL`, and `IS_REPEATABLE` are implemented for these targets with target and duplicate-use checks. Closure parameter reflection and the full Reflection API remain incomplete.
- `is_a()` tests object ancestry/interface membership and supports class-name strings when `allow_string` is true.
- `is_subclass_of()` checks parent-class and interface relationships, returns false for an object's own class, and accepts class-name strings only with `allow_string: true`.
- `property_exists()` reports declared inherited instance/static properties and present dynamic object properties regardless of the property's visibility or value. Undeclared dynamic properties on user objects can be read, written, and unset; `__get`, `__set`, and `__unset` still handle unknown properties when defined.
- Traits with property/method composition, `insteadof`, aliases, and visibility adaptations.
- Shallow object cloning with `__clone()`.
- Arithmetic, modulo, concatenation, and bitwise compound assignments on instance properties, routed through visibility/type checks and `__get`/`__set` where applicable.
- `??=` on instance properties, with short-circuit reads and writes routed through `__get`/`__set` when needed.
- Prefix and postfix increment/decrement of overloaded properties read through `__get()` and write through `__set()`; regression and `examples/classes.php` verify getter/setter order and resulting values.
- Arithmetic, modulo, concatenation, and bitwise compound assignments plus `??=` on static properties, using the declaring property's inherited storage slot and type checks.
- Magic property methods `__get`, `__set`, `__isset`, `__unset`; dynamic method hooks `__call`, `__callStatic`; `__invoke` for direct object calls and array callback APIs; and `__toString` for echo and string conversion. Under the runtime's weak scalar behavior, scalar `__toString()` results are converted to strings; array/object results raise a catchable `Error`. The compiler validates public/non-static `__invoke()` and public/non-static/zero-argument `__toString()` declarations, plus requires any declared `__toString()` return type to be `string`; complete magic-method signature validation and error timing remain incomplete.
- Reads of inaccessible typed properties through `__get()` check the returned value against the declared property type, whether the getter returns by value or by reference; incompatible values raise `TypeError`.

## Known gaps

- Intersection and DNF types, complete PHP variance rules, and complete late-static binding semantics remain partial. Method parameter and return checks resolve `self` and `parent` against the declaring class, and resolve `static` return types against the runtime called class for regular methods, static calls, parent calls, and callable arrays. Typed instance/static properties resolve `self` and `parent` against the declaring class, including trait properties against the consuming class. Other late-static contexts remain incomplete. Basic union types are supported for method parameters, returns, and properties.
- Multiple inheritance is not supported for classes; trait nesting/conflict rules and adaptation edge cases are incomplete.
- Same-named private parent/child properties now occupy distinct storage slots; methods in each declaring class and their by-reference calls select the correct slot.
- Constructor and method reference parameters/returns support variables, array elements, accessible untyped instance properties, direct static properties, and nested array offsets rooted at static properties; direct alias binding to nested static-property offsets is also supported. Private instance properties from their declaring scope and nested offsets rooted at fixed-name properties are supported. Direct references to typed properties and readonly properties raise PHP-specific catchable errors; overloaded typed-property reads through a reference-returning `__get()` check the getter's current value. Dynamic/inaccessible property paths and broader overloaded-property reference behavior remain incomplete. Destructors run at shutdown and on last-reference `unset()` for supported variable, array, object-property-array, and static-property-array slots; reassignment and cycle collection timing remain incomplete. Dynamic PHP autoload callbacks, non-class attribute targets, anonymous-class attributes, and magic methods beyond the implemented set remain missing. Unit and backed enums are supported with validation gaps. Composer PSR-4, classmap, and PSR-0 mappings are preloaded by the CLI/server when `vendor/autoload.php` is required; dynamic include return behavior remains unsupported.
- `isset()` supports multiple variables, array offsets, and object properties, including nested static or computed property names such as `isset($object->{$name}[$key])`; overloaded properties call `__isset()` before `__get()` and check returned array offsets. Fixed-name and computed dynamic property chains, including array offsets between properties, call `__isset()` at each property step and `__get()` only for intermediate properties. Property and array-offset expressions execute only after all earlier chain steps succeed; broader PHP warning/error behavior remains incomplete. Object iteration, cloning with deep/custom property semantics, and serialization are also incomplete.
- Compiler validation checks public visibility, instance/static requirements, parameter counts, and declared parameter/return types for `__serialize`, `__unserialize`, `__sleep`, `__wakeup`, `__set_state`, and `__debugInfo`. Their serialization and debug-dump behavior is not implemented yet.

The exact subset can change as implementation advances. Run the class example with `cargo run -p php-cli -- examples/classes.php` to inspect current behavior.
See [`examples/late-static-types.php`](../examples/late-static-types.php) for `self` and `parent` argument/return checks, late-bound `static` returns, and a catchable return-type failure.

Unit enums implement the built-in `UnitEnum` interface, and backed enums implement both `UnitEnum` and `BackedEnum`. `interface_exists()`, `instanceof`, `is_a()`, and `method_exists()` recognize the interfaces and their core methods (`cases`, `from`, and `tryFrom`). `ReflectionEnum` exposes `getCases()` and `isBacked()`; returned `ReflectionEnumUnitCase` / `ReflectionEnumBackedCase` objects support `getName()`, `getValue()`, and `getBackingValue()`. `getBackingType()` returns `ReflectionNamedType` for backed enums and `null` for unit enums; its name, builtin, and nullability getters are supported. Other enum reflection APIs remain incomplete.

`ReflectionClass` supports `isEnum()`, `isInterface()`, `isAbstract()`, `isFinal()`, `getParentClass()`, `isSubclassOf()`, and `implementsInterface()` for declared types. `getParentClass()` returns a reflection object for a declared parent or `false` for a root class. Other class inspection methods remain incomplete.

Attributes on parameters of named functions, methods, closures, and arrow functions are retained. `ReflectionFunction::getParameters()` now exposes closure and arrow-function parameters, whose attributes can be filtered, inspected, checked for repetition, and instantiated through `ReflectionParameter` and `ReflectionAttribute`.
