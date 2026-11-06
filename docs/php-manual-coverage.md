# PHP manual coverage audit

Audit date: 2026-10-31. Compatibility baseline: **PHP 8.5.12**, the latest stable PHP release on the audit date; PHP 8.6 release candidates are excluded. The repo itself does not declare a PHP compatibility target. See the [official PHP downloads](https://www.php.net/downloads.php) and [release archive](https://www.php.net/archive/2026.php).

## Scope and method

The comparison follows the PHP 8.5-era [PHP Manual contents](https://www.php.net/manual/en/), especially its [Language Reference](https://www.php.net/manual/en/langref.php), [internal-function overview](https://www.php.net/manual/en/functions.internal.php), and [extension categorization](https://www.php.net/manual/en/extensions.php). The manual covers the language, runtime features, and a function reference organized by extension; its extension appendix documents more than 150 extensions. APIs removed before PHP 8.5 are classified as legacy extras rather than missing support.

The local side was checked against `README.md`, the feature guides in `docs/`, syntax/AST/compiler/runtime dispatch and the active [compatibility backlog](php-compatibility-backlog.md). Every top-level manual section and extension category is classified below. The common core families (strings, arrays, math, filesystem, date/time, JSON, errors and serialization) include concrete API gaps; individual signatures and overloads across all 150+ optional extensions have not yet been reconciled one by one. This distinction keeps the audit useful without claiming a complete function-by-function comparison that has not been done.

## Findings

### 1. Language reference — substantial gaps

| Manual area | Current project coverage | Gaps to track |
| --- | --- | --- |
| Basic syntax | PHP open/close tags are stripped; `//`, `#`, and `/* ... */` comments including PHPDoc; decimal literals; single/double-quoted strings | Mixed HTML/PHP output, `<?=`, heredoc/nowdoc, variable interpolation in strings, binary/octal/hex literals and numeric separators, `__halt_compiler`, full lexical/grammar compatibility |
| Constants and magic constants | Top-level `const` declarations with scalar/array/arithmetic/class-constant values and references to other declared constants; `define()`/`defined()`/`constant()`, class/interface constants, `__LINE__`, `__NAMESPACE__`, contextual `__CLASS__`/`__FUNCTION__`/`__METHOD__`; common predefined constants including `PHP_EOL`, integer and float limits/precision, and rounding modes; `__FILE__`/`__DIR__` only in static include path resolution | General expression expansion for `__FILE__`/`__DIR__`; `__TRAIT__`, `__PROPERTY__`; remaining predefined constants across PHP/extensions; conditional include timing for included constants is partial |
| Types and coercion | Scalars, `null`/`true`/`false`, `mixed`, `void`, arrays/objects, `iterable` accepts arrays, generators, and `Traversable` objects; `is_iterable()` checks the same set, unions, intersections and parenthesized DNF in supported parameter/return/property contexts, selected weak coercions | Complete `never` behavior, `callable` and resource values, complete nominal coverage of built-in interfaces, `self`/`parent`/`static` return types, strict typing mode and full numeric-string/key coercion rules |
| Expressions/operators | Arithmetic, modulo, power, assignment, bitwise, logical, relational and spaceship `<=>` comparisons, null coalescing, ternary, calls and selected references | Error-control `@`, shell execution backticks, remaining array/object comparison edge cases, and complete expression grammar |
| Control structures | `if`, loops, array `foreach`, `switch`, strict `match` expressions, `try`/`catch`/`finally`, `goto` labels with loop/switch-entry validation, computed `break`/`continue`, and basic `exit`/`die` termination; `declare(strict_types=0|1)` and scalar strict typing | Other `declare` directives, PHP 8.4 `exit()` named/callable semantics and exact type diagnostics, arbitrary `Traversable` edge cases, full alternate syntax, remaining switch semantics; `UnhandledMatchError` message fidelity is partial |
| Functions | User functions, methods, closures, arrows, generators, named/variadic arguments, first-class callables and several reference forms | Full callable/reference grammar, variable functions and dynamic methods in every form, all PHP argument/type edge cases, complete named-argument behavior for internal functions, complete `Generator` contract |
| Classes and objects | Classes, inheritance, interfaces, traits, visibility, typed/readonly properties, untyped class constants, cloning, enums, attributes, anonymous classes with constructor arguments, destructors, `__get`, `__set`, `__isset`, `__unset`, `__call`, `__callStatic`, `__invoke`, `__toString`, automatic `Stringable` recognition, `__clone`; nested `isset($object->property[$key])` dispatches `__isset` before `__get`; same-named private parent/child properties use separate slots | Property hooks, typed class constants, lazy objects, serialization, iteration, object comparison and late-static/type rules; private-property reference paths are partial; magic `__debugInfo`, `__set_state`, `__serialize`, `__unserialize`, `__sleep`, `__wakeup` |
| Namespaces | Multiple namespace blocks, imports and selected fallback rules | Full name-resolution and dynamic-language edge cases, runtime autoload callback behavior and complete include/autoload integration |
| Errors, exceptions and diagnostics | Explicit throwable flow, selected built-in exception classes, typed catches and `finally`; `error_reporting()` masks warnings/deprecations, `error_get_last()`/`error_clear_last()` expose and reset their source-located last-error state | Custom error handlers, remaining diagnostic severities, conversion of runtime failures to PHP `Error` subclasses, full exception hierarchy and complete source/trace parity |
| Fibers and generators | Generators and `yield from` for arrays, generators, user `Iterator` objects, and nested `IteratorAggregate` results | Fibers are absent; generator lifecycle edge cases and some delegated iterator behaviors remain |
| Attributes and predefined symbols | A limited set of constants, superglobals, `Throwable` hierarchy, `Countable` interface discovery, `count()` dispatch, and required method-shape checks (tentative return-type diagnostics remain partial), and core runtime objects; `JsonSerializable::jsonSerialize()` is recognized for JSON encoding; `Stringable` is recognized for explicit and automatic `__toString()` implementations | Attribute syntax/reflection and predefined attributes; predefined `Iterator`, `IteratorAggregate`, `InternalIterator`, `ArrayAccess`, `Serializable`, `Fiber`, `WeakReference`, `WeakMap`, `UnitEnum`, `BackedEnum`, and `SensitiveParameterValue` contracts |
| References and variables | Globals, locals/statics, aliases and selected argument/return/array references | Complete copy-on-write, symbol insertion order, `$GLOBALS` whole-table semantics, all reference contexts, reference diagnostics |

The local detail pages describe these limits in [syntax/parser](syntax-and-parser.md), [control flow](control-flow.md), [OOP](oop-compatibility.md), [variables](variables.md), [references](references.md), [functions](functions.md), [namespaces](namespaces.md), and [exceptions](exceptions.md). The specific remaining work from this table is tracked in the backlog, but the table adds manual sections that were not represented there before, notably HTML/PHP mode, heredoc/nowdoc, `goto`, `declare`, fibers, property hooks, lazy objects, and diagnostic error handling.

### 2. Core and standard functions — broad coverage gap

The project has hand-implemented families for selected arrays, strings, JSON, symbol-table helpers, type predicates, HTTP/server functions, and exception introspection. It does not provide a drop-in implementation of PHP's internal function reference. Major missing or only very partial manual families include:

- filesystem/path, stream contexts and wrappers, file metadata and permissions;
- date/time, timezone and calendar APIs;
- broad math, random-number and number-formatting APIs;
- multibyte/string encoding, regular expressions, tokenization and locale APIs;
- process, signal, environment and shell APIs;
- compression/archive and cryptography APIs;
- XML, DOM, SimpleXML, SOAP and other web-service APIs;
- image, mail, authentication, internationalization and platform-specific APIs;
- PHP runtime/configuration/introspection APIs, error handlers, output buffering and reflection.

This is a family-level finding only. The exact missing function names, signatures, named-argument labels, defaults, return types, flags, version availability, warnings and exception behavior still need an API-by-API inventory against the [official function index](https://www.php.net/manual/en/indexes.functions.php). The manual notes that internal APIs are split across extensions and that extension-dependent calls may not exist in a given PHP build ([internal-function overview](https://www.php.net/manual/en/functions.internal.php)).

Concrete examples verified against runtime dispatch: `file_get_contents()` accepts local paths and `php://input` with offset and length reads; metadata and access checks include `file_exists()`, `is_file()`, `is_dir()`, `is_link()`, `is_readable()`, `is_writable()`, `is_executable()`, `filesize()`, `realpath()`, `getcwd()`, and path parsing with `basename()`/`dirname()`. Local filesystem operations now include `mkdir()`, `rmdir()`, `copy()` and `rename()` with document-root confinement. Directory listing and handle-based iteration are available through `scandir()` and `opendir()`/`readdir()`/`rewinddir()`/`closedir()`; stream contexts remain absent. `parse_str()` builds nested result arrays from query strings; `preg_match()`, timezone configuration, `ob_start()`, `serialize()` and `unserialize()` are absent. Date functions include `date()`, `strtotime()`, and deprecated libc-backed `strftime()`/`strptime()` variants. User diagnostics include `set_error_handler()`, `restore_error_handler()`, `trigger_error()`, `error_reporting()`, `error_get_last()`, and `error_clear_last()`, though exact PHP parity remains partial. These examples illustrate the gap and are not a complete missing-function list. Compare the official [string](https://www.php.net/manual/en/ref.strings.php), [array](https://www.php.net/manual/en/ref.array.php), [math](https://www.php.net/manual/en/ref.math.php), [filesystem](https://www.php.net/manual/en/ref.filesystem.php), and [date/time](https://www.php.net/manual/en/ref.datetime.php) indexes for the size of these API families.

The runtime currently exports two callback-function names that are not in PHP's official array-function index: `array_udiff_ukey()` and `array_uintersect_ukey()`. The [array guide](array-compatibility.md) now labels them as project extensions; they should not count toward PHP API coverage.

### 2.1 Concrete API gaps checked in core families

The following names were checked against the local builtin dispatch and the corresponding official reference indexes. They are examples selected for a practical compatibility target; deprecated, platform-specific, and niche functions still need their own priority decision.

| PHP reference family | Implemented subset found locally | Specific missing APIs / behavior |
| --- | --- | --- |
| Strings | The functions listed in [function support](functions.md), including trim/split/search/replace/compare, HTML escaping, `htmlentities()`, `html_entity_decode()`, and `get_html_translation_table()` for the HTML 4.01 entity table, `str_pad`, slash escaping, `str_rot13`, `stristr()`, `strpbrk()`, `str_word_count()`, `bin2hex()`/`hex2bin()`, `base64_encode()`/`base64_decode()`, `crc32()`, `hash_equals()`, `hash()`/`hash_file()` for MD5/SHA-1/SHA-224/SHA-256/SHA-384/SHA-512/SHA3-224/SHA3-256/SHA3-384/SHA3-512/CRC32B and `hash_algos()`, and `hash_hmac()`/`hash_hmac_file()`/`hash_hmac_algos()`/`hash_pbkdf2()`/`hash_hkdf()` for MD5/SHA-1, `md5()`/`sha1()` and their file variants, `str_getcsv()`, `strtok()`, `levenshtein()`/`similar_text()`, `chr()`/`ord()`/`count_chars()`, `nl2br()`, `sprintf()`/`vsprintf()` common format subset, `printf()`/`vprintf()` output, `fprintf()` stream output, `parse_str()` query parsing, `wordwrap()`, `substr_replace()`/`substr_compare()`, `addcslashes()`/`stripcslashes()`, `strip_tags()`, and `number_format()` | Other file/hash variants. Also missing are broad PCRE (`preg_*`) and multibyte (`mb_*`) families. |
| Arrays | Most documented core array function families and newer helpers (`array_all`, `array_any`, `array_find*`, `array_first`, `array_last`) are present, including the `pos()` alias | Runtime exposes `array_udiff_ukey()` and `array_uintersect_ukey()` although they are not PHP standard functions, and `each()` although PHP removed it in 8.0; the array guide labels these as nonstandard/legacy extras. Reference-required callbacks, `Traversable` callback inputs, copy-on-write, key-casting rules, comparator coercion, sort flags, diagnostics and PHP comparison rules remain partial. |
| Math | Arithmetic operators and selected array aggregations; common rounding, roots, division/remainder, exponential/logarithmic, trigonometric/hyperbolic, angle conversion, `hypot`, finite/infinite/NaN, scalar/array `min`/`max`, `base_convert`, `decbin`, `dechex`, and `decoct` are implemented | Random APIs, `number_format` variants other than `number_format()`, constants, full object comparison, and exact coercion/diagnostic behavior in the [official math index](https://www.php.net/manual/en/ref.math.php). |
| Filesystem / streams | `file()` line arrays and `file_get_contents()` on local paths and `php://input` with offset/length, local and temporary `fopen()` streams, `file_put_contents()`, `tempnam()`, `sys_get_temp_dir()`, `unlink()`, `touch()` timestamps, `chmod()`, `disk_free_space()`/`diskfreespace()`, `disk_total_space()`, `copy()`, `rename()`, `mkdir()`, `rmdir()`, `scandir()`, `opendir()`/`readdir()`/`rewinddir()`/`closedir()` handles (including PHP 8.5 null-handle deprecations), metadata/access checks including `filesize()`, `filemtime()`, `fileatime()`, `filectime()`, `fileinode()`, `fileowner()`, `filegroup()`, `fileperms()`, `filetype()`, `stat()`, and `lstat()`, path parsing, upload checks/move, and `php://temp` / `php://memory` resources | Locks, stream contexts, filters, and broader wrapper behavior. Local path mutations are confined to the configured document root; see the [filesystem index](https://www.php.net/manual/en/ref.filesystem.php) and [wrapper list](https://www.php.net/manual/en/wrappers.php). |
| Date/time | `time()`, `microtime()`, `hrtime()`, `gettimeofday()`, `date()`/`gmdate()`, deprecated libc-backed `strftime()`/`gmstrftime()` and Unix `strptime()`, `getdate()`, `localtime()`, `idate()`, `checkdate()`, `mktime()`/`gmmktime()`, and partial `strtotime()` | Timezone configuration and transition APIs, `DateTime`, `DateTimeImmutable`, `DateInterval`, `DatePeriod`, `DateTimeZone`, locale/calendar, broad parsing and formatting APIs. See the [date/time index](https://www.php.net/manual/en/ref.datetime.php). |
| Regex, serialization, output and error APIs | User diagnostic subset: `set_error_handler()`, `restore_error_handler()`, `trigger_error()`/`user_error()`, `error_reporting()`, `error_get_last()`, and `error_clear_last()` | `preg_*`; `serialize`/`unserialize` and object serialization hooks; `ob_*` output buffering; remaining diagnostic severity, formatting, handler and exception parity. |

Array names need a final source-to-manual reconciliation: the existing local guide lists implemented functions, but several callback-oriented names and edge semantics have changed across PHP versions. The official array reference page currently includes newer helpers such as `array_all`, `array_any`, `array_find`, `array_find_key`, `array_first`, and `array_last`, which the local inventory already lists; name presence alone does not prove matching signatures or semantics.

### 3. Extension reference — not generally implemented

The project is a focused runtime rather than a PHP build with the manual's extension catalog. It currently contains a JSON implementation, PDO/database abstractions with a limited set of drivers/APIs, and selected web/session/upload support in its server. The manual's extension list covers more than 150 documented extensions. Most of those extension families have no equivalent here, including GD/ImageMagick, cURL, OpenSSL/cryptography, mbstring/intl, PCRE, XML/DOM/SimpleXML, SOAP, Zip, Zlib, LDAP, IMAP, sockets, shared memory, process-control APIs and many platform-specific modules.

Even where an extension family exists (JSON, PDO, database drivers, sessions), implementation of the family name does not imply parity with every documented function, flag, error code, configuration directive or edge case. See [database limits](database.md), [web server limits](web-server.md), and [function limits](functions.md).

| Official manual category | Project status |
| --- | --- |
| Affecting PHP behaviour / runtime configuration | Selected `.ini` settings only; no full PHP configuration or error-reporting surface |
| Audio, image processing, GUI | Absent |
| Authentication, mail, search-engine integrations | Absent |
| CLI | Project CLI and `$argc`/`$argv` exist; PHP's full CLI option, stream and process contracts are absent |
| Compression/archive | Absent |
| Cryptography | Absent |
| Database | PDO-like layer and PostgreSQL/MySQL/ODBC/MongoDB drivers exist; API and backend parity remains partial |
| Date/time | `time()`, `microtime()`, `hrtime()`, calendar formatting/parsing helpers, and deprecated libc-backed `strftime()`/`strptime()`; timezone configuration/transitions and DateTime APIs absent |
| Filesystem | Local file reads/writes, copy/rename/directory create/remove, line reads, basic file streams, metadata/access checks, temporary-file helpers, and upload APIs; wrapper and full filesystem parity remains absent |
| Human language/encoding | UTF-8 runtime strings with byte-oriented selected helpers; mbstring/intl/locale support absent |
| Non-text MIME output | Selected HTTP response headers/status only |
| Other Services | Absent; no parity for service APIs such as cURL, DNS, LDAP, or mail transports beyond the selected server features |
| Process control | Absent |
| Other basic extensions | JSON is partial; encoding includes `JSON_PARTIAL_OUTPUT_ON_ERROR` substitutions, error tracking, and PHP-compatible depth argument ranges; other error edges and detailed UTF-8 classification remain open; most extensions in this category are absent |
| Server-specific / session | Prototype HTTP server and selected session behavior; no PHP SAPI/FPM contract |
| Text processing | Selected string helpers; PCRE and most string/encoding APIs absent |
| Variable/type | Selected symbol-table and type predicates, including `is_countable()`; reflection is partial and broad introspection absent |
| Web services | Absent |
| Windows-only | Absent |
| XML manipulation | Absent |

### 4. PHP runtime and deployment features — partial or absent

The manual also documents installation/configuration, php.ini directives, FPM, request lifecycle, stream wrappers, sessions, uploads, connection handling, garbage collection and security. This repository has a standalone CLI and a prototype HTTP server with selected PHP-style request data, session and upload behavior. It is not PHP-FPM, does not support the complete PHP SAPI/configuration surface, and does not implement the full wrapper/stream, error-reporting, security or garbage-collection contracts documented by PHP.

| PHP Manual chapter | Project status and missing behavior |
| --- | --- |
| Getting started / installation | Rust CLI install script and Linux package exist; PHP's Unix/macOS/Windows builds, web server module installs, PECL/PIE extensions and PHP-FPM setup are not replicated. |
| Runtime configuration | A selected `.ini` subset is parsed and a partial error-reporting/custom-handler API exists; PHP's full directive catalog, per-directory configuration, environment/config precedence, extension loading and `ini_*` API are absent or partial. |
| Security | HTTP document-root traversal and symlink escapes are guarded by this server; PHP's CGI/Apache SAPI, session security, filesystem/database configuration, error disclosure and user-input contracts are not equivalent. |
| HTTP authentication / cookies | No HTTP-auth API; server parses request cookies, but PHP `setcookie()`/`setrawcookie()` behavior is absent. |
| Sessions | Selected start/destroy/status/id operations and request session persistence exist; full handler registration, cookie/config options, locking, shared storage guarantees and PHP serialization behavior are not complete. |
| File uploads | Multipart parsing, size limits, temporary files and selected move/check functions exist; all PHP upload error codes, nested-name normalization, ini limits and SAPI edge behavior need parity checks. |
| Remote files / wrappers | `php://input` has a narrow implementation; PHP's `file://`, `http(s)://`, `data://`, `php://filter`, compression, FTP and user-defined wrappers are absent. |
| Connection handling | HTTP keep-alive and response serialization exist at server level; PHP connection state APIs and SAPI lifecycle behaviors are absent. |
| Persistent DB connections | The configured shared pool is the current model; PHP PDO persistence attributes and per-process/request lifecycle are not equivalent to all PHP drivers. |
| CLI | Script path, arguments and exit codes work for the project CLI; PHP's full CLI option set, interactive mode, stdin wrapper and PHP binary invocation contract are absent. |
| Garbage collection / DTrace | PHP cycle collection APIs, GC controls and DTrace probes are absent. |
| Appendices / introspection lists | No parity audit exists for PHP's reserved-word list, parser tokens, resource types, filter list, socket transports, function aliases, type-comparison tables or full php.ini directive appendix. |

## Priority order for a useful compatibility target

Trying to implement every page in the manual means supporting hundreds of optional/platform-specific extension APIs. For this audit I fixed the language baseline at PHP 8.5.12. The remaining implementation work can now be made concrete in this order:

1. Finish the PHP 8.5 language reference and predefined core types/interfaces.
2. Reconcile every core function name, signature, named parameter, flag, return behavior and version against the manual.
3. Implement and verify the common core APIs (strings, arrays, math, filesystem, date/time, error handling, serialization, reflection).
4. For each extension family, record whether it is in scope; then reconcile every API for the selected JSON, PDO/database, session, upload and filesystem families.
5. Compare runtime dispatch with `function_exists()` and callback dispatch, and add PHP 8.5 differential tests for supported APIs.

Current status: **the project implements a useful but narrow PHP subset. The manual sections and extension categories are classified here; function-by-function reconciliation across every optional extension remains incomplete.**
